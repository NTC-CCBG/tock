// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

use core::cell::Cell;
use cortexm4f::support::atomic;
use kernel::hil::time::{
    Alarm, AlarmClient, Counter, Freq16KHz, OverflowClient, Ticks, Ticks32, Time,
};
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, ReadWrite};
use kernel::utilities::StaticRef;
use kernel::{debug, ErrorCode};

use crate::clock::{Clock, HighClocks};
use crate::nvic;

// Clock frequencies (Hz)
const LFCG_CORE_CLK: u32 = 32_768; // 32KHz low-frequency clock

// Maximum cycle times
const MAX_CYCLE_TIME_MS: u32 = 200_000;
const MAX_CYCLE_TIME_US: u32 = 200_000;

/// Clock source selection for the timer
#[derive(Copy, Clone, PartialEq)]
pub enum SourceClock {
    /// APB2 clock
    Apb2,
    /// 32KHz low-frequency clock
    Lfcg32K,
}

/// Internal 8-bit/16-bit/32-bit Timer (ITIM32)
#[repr(C)]
struct ITimRegisters {
    /// Internal 8-Bit Timer Counter (Offset 00h)
    cnt: ReadWrite<u8, CNT_8::Register>,
    /// Internal Timer Prescaler (Offset 01h)
    pre: ReadWrite<u8, PRE_8::Register>,
    /// Internal 16-Bit Timer Counter (Offset 02h)
    cnt16: ReadWrite<u16, CNT_16::Register>,
    /// Internal Timer Control and Status (Offset 04h)
    cts: ReadWrite<u8, CTS::Register>,
    /// Reserved (Offset 05h-06h)
    _reserved0: [u8; 2],
    /// ITIM32 Version Register (Offset 07h)
    ver: ReadWrite<u8, VER::Register>,
    /// Internal 32-Bit Timer Counter (Offset 08h)
    cnt32: ReadWrite<u32, CNT_32::Register>,
}

register_bitfields![u8,
    CNT_8 [
        /// 8-bit Timer Counter Value
        CNT OFFSET(0) NUMBITS(8) []
    ],
    PRE_8 [
        /// 8-bit Timer Prescaler Value
        PRE OFFSET(0) NUMBITS(8) []
    ],
    CTS [
        /// Internal Timer Enable
        ITEN OFFSET(7) NUMBITS(1) [],
        /// Clock Select
        CKSEL OFFSET(4) NUMBITS(1) [],
        /// Timeout Wake-Up Enable
        TO_WUE OFFSET(3) NUMBITS(1) [],
        /// Timeout Interrupt Enable
        TO_IE OFFSET(2) NUMBITS(1) [],
        /// Timeout Status
        TO_STS OFFSET(0) NUMBITS(1) []
    ],
    VER [
        /// ITIM32 Version (Read-only, returns 04h for this version)
        ITIM32_VER OFFSET(0) NUMBITS(8) []
    ]
];

register_bitfields![u16,
    CNT_16 [
        /// 16-bit Timer Counter Value
        CNT OFFSET(0) NUMBITS(16) []
    ]
];

register_bitfields![u32,
    CNT_32 [
        /// 32-bit Timer Counter Value
        CNT OFFSET(0) NUMBITS(32) []
    ]
];

const ITIM1_BASE: StaticRef<ITimRegisters> =
    unsafe { StaticRef::new(0x400B_0000 as *const ITimRegisters) };

pub struct Itim<'a> {
    registers: StaticRef<ITimRegisters>,
    clock: OptionalCell<&'a Clock>,
    client: OptionalCell<&'a dyn AlarmClient>,
    irqn: u32,
    alarm_value: OptionalCell<u32>,
    measure_start_cnt: OptionalCell<u32>,
    // Software counter for tracking current time (updated on each alarm fire)
    current_time: OptionalCell<u32>,
    clock_num: HighClocks,
    // Hardware ticks per Tock tick (for prescaler compensation)
    hw_ticks_per_tock_tick: Cell<u32>,
}

impl<'a> Itim<'a> {
    pub const fn new(clock_num: HighClocks) -> Self {
        Self {
            registers: ITIM1_BASE,
            clock: OptionalCell::empty(),
            client: OptionalCell::empty(),
            irqn: nvic::ITIM32_1,
            alarm_value: OptionalCell::empty(),
            measure_start_cnt: OptionalCell::empty(),
            current_time: OptionalCell::empty(),
            clock_num,
            hw_ticks_per_tock_tick: Cell::new(1),
        }
    }

    pub fn set_clock(&self, clock: &'a Clock) {
        self.clock.set(clock);
    }

    /// Get the APB2 clock frequency in Hz
    fn get_apb2_clock_freq(&self) -> u32 {
        self.clock
            .get()
            .and_then(|clk| clk.get_clock_source(self.clock_num))
            .unwrap_or(100_000_000) // Default to 100MHz if clock not set
    }

    /// Calculate and set PRE and CNT registers for millisecond timing
    /// Returns Ok(()) on success, Err(ErrorCode::INVAL) if millisec is invalid
    fn set_regs_by_millisec(&self, clock: SourceClock, millisec: u32) -> Result<(), ErrorCode> {
        // Validate input
        if millisec == 0 || millisec > MAX_CYCLE_TIME_MS {
            return Err(ErrorCode::INVAL);
        }

        let (pre, cnt) = match clock {
            SourceClock::Apb2 => {
                let apb2_clk = self.get_apb2_clock_freq();
                let pre = 128u32;
                let time_base = apb2_clk / pre;
                let cnt = (time_base / 1000) * millisec;
                (pre, cnt)
            }
            SourceClock::Lfcg32K => {
                let pre = 16u32;
                let time_base = LFCG_CORE_CLK / pre;
                let cnt = (time_base / 1000) * millisec;
                (pre, cnt)
            }
        };

        // Set PRE and CNT registers (hardware expects values minus 1)
        self.registers.pre.set((pre - 1) as u8);
        self.registers.cnt32.set(cnt - 1);

        Ok(())
    }

    /// Calculate and set PRE and CNT registers for microsecond timing
    /// Returns Ok(()) on success, Err(ErrorCode::INVAL) if microsec is invalid
    fn set_regs_by_microsec(&self, microsec: u32) -> Result<(), ErrorCode> {
        // Validate input
        if microsec == 0 || microsec > MAX_CYCLE_TIME_US {
            return Err(ErrorCode::INVAL);
        }

        let apb2_clk = self.get_apb2_clock_freq();
        // Use prescaler of 1 for maximum resolution
        let pre = 1u32;
        let time_base = apb2_clk / pre;
        let cnt = (time_base / 1_000_000) * microsec;

        // Set PRE and CNT registers (hardware expects values minus 1)
        self.registers.pre.set((pre - 1) as u8);
        self.registers.cnt32.set(cnt - 1);

        Ok(())
    }

    // Initializes the timer for one-shot alarm mode
    // ITIM is a down-counter without compare registers, so we use it as a one-shot timer:
    // - Software counter (current_time) tracks the monotonic time
    // - When set_alarm() is called, load CNT32 with ticks-until-alarm
    // - When CNT32 reaches 0, interrupt fires and we update current_time
    fn start_counter(&self) {
        // Ensure timer is disabled before modifying registers
        self.registers.cts.modify(CTS::ITEN::CLEAR);

        // Wait until the module is actually disabled
        while self.registers.cts.is_set(CTS::ITEN) {}

        // Select APB2 clock source
        self.registers.cts.modify(CTS::CKSEL::CLEAR);

        // Calculate prescaler to achieve a frequency close to 16KHz from APB2 clock
        // PRE_8 is only 8-bit (max value 256), so we need a two-stage division
        // Hardware formula: hw_freq = input_freq / (PRE_8 + 1)
        // Target: 16KHz for Tock, but PRE_8 max is 256
        let apb2_clk = self.get_apb2_clock_freq();
        let target_freq = 16_000u32;
        let total_divisor = apb2_clk / target_freq;

        // Find best PRE_8 value (max 256) and calculate remaining divisor
        // We want: apb2_clk / (pre + 1) / scale_factor ≈ 16kHz
        // Choose pre to give a convenient intermediate frequency
        // Example:
        //   APB2 clock: 96,000,000 Hz
        //   Target:     16,000 Hz
        //   Divisor:    6,000 = 256 × 23.4 ≈ 256 × 24
        //   PRE_8 = 255 → hardware frequency = 375kHz
        //   Scale = 24  → effective Tock frequency ≈ 15.625kHz (close to 16kHz)
        let pre = if total_divisor <= 256 {
            // Can achieve 16kHz directly with prescaler
            self.hw_ticks_per_tock_tick.set(1);
            (total_divisor - 1) as u8
        } else {
            // Need two-stage division
            // Find factors of total_divisor that work with 8-bit prescaler
            // total_divisor = pre * scale
            // Choose pre close to 256 for best resolution
            let pre = 256u32.min(total_divisor);
            let scale = (total_divisor + pre - 1) / pre; // Round up
            self.hw_ticks_per_tock_tick.set(scale);
            (pre - 1) as u8
        };

        self.registers.pre.set(pre);

        debug!(
            "[Itim] start_counter: apb2_clk={} Hz, target=16kHz, divisor={}, pre={}, scale={}",
            apb2_clk,
            total_divisor,
            pre,
            self.hw_ticks_per_tock_tick.get()
        );

        // Initialize software time counter to 0
        self.current_time.set(0);

        // Timer is configured but not started
        // Will be started when set_alarm() is called
    }
    pub fn is_enabled_clock(&self) -> bool {
        // self.clock.is_enabled()
        true
    }

    pub fn enable_clock(&self) {
        // self.clock.enable();
    }

    pub fn disable_clock(&self) {
        // self.clock.disable();
    }

    /// Start wake-up event timer
    /// Generates a wake-up event to the PMC using 32KHz clock
    pub fn start_wakeup_timer(&self, millisec: u32) -> Result<(), ErrorCode> {
        // Stop timer first
        self.stop()?;

        // Select 32K clock source
        self.registers.cts.modify(CTS::CKSEL::SET);

        // Set PRE and CNT registers
        self.set_regs_by_millisec(SourceClock::Lfcg32K, millisec)?;

        // Enable interrupt
        self.registers.cts.modify(CTS::TO_IE::SET);

        // Enable timeout wake-up
        self.registers.cts.modify(CTS::TO_WUE::SET);

        // Enable the module
        self.registers.cts.modify(CTS::ITEN::SET);

        Ok(())
    }

    /// Block delay for given microseconds
    /// This function blocks execution until the timer completes
    /// Needs around 16us execution time overhead
    pub fn delay_microsec(&self, microsec: u32) -> Result<(), ErrorCode> {
        // Stop timer first
        self.stop()?;

        // Set source clock to APB2 clock
        self.registers.cts.modify(CTS::CKSEL::CLEAR);

        // Set PRE and CNT registers
        self.set_regs_by_microsec(microsec)?;

        // Enable the module (without interrupt)
        self.registers.cts.modify(CTS::ITEN::SET);

        // Block waiting until timer count down to 0
        while !self.registers.cts.is_set(CTS::TO_STS) {}

        // Stop timer
        self.stop()
    }

    /// Measure time in microseconds
    /// Call with start=true to begin measurement, start=false to get elapsed time
    /// Returns elapsed microseconds on stop, 0 on start
    pub fn measure_time_microsec(&self, start: bool) -> u32 {
        if start {
            // Ensure timer is disabled before modifying registers (per hardware spec)
            self.registers.cts.modify(CTS::ITEN::CLEAR);
            while self.registers.cts.is_set(CTS::ITEN) {}

            // Set source clock to APB2
            self.registers.cts.modify(CTS::CKSEL::CLEAR);

            // Start measurement
            // Use microsecond timing (100ms = 100,000us)
            let _ = self.set_regs_by_microsec(100_000);

            // Enable the module (without interrupt)
            self.registers.cts.modify(CTS::ITEN::SET);

            // Read initial counter value after starting
            let start_cnt = self.registers.cnt32.get();
            self.measure_start_cnt.set(start_cnt);

            0
        } else {
            // Read current counter value first (while timer is still running)
            let end_cnt = self.registers.cnt32.get();
            let pre = self.registers.pre.get() as u32 + 1;

            // Stop timer
            let _ = self.stop();

            // Calculate time base frequency (Hz)
            let apb2_clk = self.get_apb2_clock_freq();
            let time_base_hz = apb2_clk / pre;

            // Get start count (default to 0 if not set)
            let start_cnt = self.measure_start_cnt.take().unwrap_or(0);

            // Calculate elapsed ticks (timer counts down from start_cnt)
            let elapsed_ticks = start_cnt.saturating_sub(end_cnt);

            // Convert to microseconds: elapsed_ticks / time_base_hz * 1000000
            // Use 64-bit to avoid overflow: (ticks * 1000000) / freq
            let elapsed_us_64 = (elapsed_ticks as u64 * 1_000_000u64) / time_base_hz as u64;

            elapsed_us_64 as u32
        }
    }

    pub fn get_cnt32(&self) -> u32 {
        self.registers.cnt32.get()
    }

    /// Read the ITIM32 version register
    /// Returns 04h for the current version
    pub fn get_version(&self) -> u8 {
        self.registers.ver.get()
    }

    /// Check if timeout status bit is set (timer has reached zero)
    pub fn is_timeout(&self) -> bool {
        self.registers.cts.is_set(CTS::TO_STS)
    }

    pub fn handle_interrupt(&self) {
        // CRITICAL: Disable the timer FIRST to prevent auto-restart
        // Hardware spec: timer automatically restarts counting after timeout
        // We want one-shot mode, so stop it immediately
        self.registers.cts.modify(CTS::ITEN::CLEAR);

        // Wait until the module is actually disabled
        while self.registers.cts.is_set(CTS::ITEN) {}

        // Now safe to clear timeout status
        self.registers.cts.modify(CTS::TO_STS::SET);

        // Update current_time to the alarm value (we've reached the alarm time)
        if let Some(alarm_val) = self.alarm_value.take() {
            self.current_time.set(alarm_val);

            // Disable interrupt
            self.registers.cts.modify(CTS::TO_IE::CLEAR);

            // Fire the alarm callback
            self.client.map(|client| client.alarm());
        }
    }

    pub fn finalize(&self) {
        // Enable clock if needed
        self.enable_clock();

        // Start the counter for alarm functionality
        self.start_counter();
    }
}

impl Time for Itim<'_> {
    type Frequency = Freq16KHz;
    type Ticks = Ticks32;

    fn now(&self) -> Self::Ticks {
        // Return software-tracked time
        // When alarm fires, current_time is updated to the alarm value
        // Between alarms, we estimate based on how much the counter has counted down
        let base_time = self.current_time.get().unwrap_or(0);

        // If alarm is set, add elapsed ticks since last update
        if self.is_armed() {
            if let Some(alarm_val) = self.alarm_value.get() {
                let hw_cnt = self.registers.cnt32.get();
                let tock_ticks_until_alarm = alarm_val.wrapping_sub(base_time);
                let hw_ticks_until_alarm =
                    tock_ticks_until_alarm * self.hw_ticks_per_tock_tick.get();

                // Convert hardware counter to Tock ticks
                // Hardware counts down, so elapsed = initial - current
                if hw_cnt <= hw_ticks_until_alarm {
                    let hw_elapsed = hw_ticks_until_alarm - hw_cnt;
                    let tock_elapsed = hw_elapsed / self.hw_ticks_per_tock_tick.get();
                    Self::Ticks::from(base_time.wrapping_add(tock_elapsed))
                } else {
                    Self::Ticks::from(base_time)
                }
            } else {
                Self::Ticks::from(base_time)
            }
        } else {
            Self::Ticks::from(base_time)
        }
    }
}

impl<'a> Counter<'a> for Itim<'a> {
    fn set_overflow_client(&self, _client: &'a dyn OverflowClient) {}

    // starts the timer
    fn start(&self) -> Result<(), ErrorCode> {
        self.start_counter();

        Ok(())
    }

    fn stop(&self) -> Result<(), ErrorCode> {
        // Disable the module
        self.registers.cts.modify(CTS::ITEN::CLEAR);

        // Wait until the module is disabled (can take several clocks)
        while self.registers.cts.is_set(CTS::ITEN) {}

        // Disable interrupt
        self.registers.cts.modify(CTS::TO_IE::CLEAR);

        // Clear timeout status
        self.registers.cts.modify(CTS::TO_STS::SET);

        Ok(())
    }

    fn reset(&self) -> Result<(), ErrorCode> {
        // Ensure timer is disabled before modifying CNT32 (per hardware spec)
        self.registers.cts.modify(CTS::ITEN::CLEAR);
        while self.registers.cts.is_set(CTS::ITEN) {}

        // Note: Hardware spec requires CNT_32 minimum value of 1, but we set 0 here
        // for reset. The timer should be reconfigured before use.
        self.registers.cnt32.set(0);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.registers.cts.is_set(CTS::ITEN)
    }
}

impl<'a> Alarm<'a> for Itim<'a> {
    fn set_alarm_client(&self, client: &'a dyn AlarmClient) {
        self.client.set(client);
    }

    fn set_alarm(&self, reference: Self::Ticks, dt: Self::Ticks) {
        let mut expire = reference.wrapping_add(dt);
        let now = self.now();
        if !now.within_range(reference, expire) {
            expire = now;
        }

        if expire.wrapping_sub(now) <= self.minimum_dt() {
            expire = now.wrapping_add(self.minimum_dt());
        }

        let _ = self.disarm();

        // Ensure timer is disabled before modifying CNT32 register (per hardware spec)
        self.registers.cts.modify(CTS::ITEN::CLEAR);

        // Wait until the module is actually disabled
        while self.registers.cts.is_set(CTS::ITEN) {}

        // Save the alarm value
        self.alarm_value.set(expire.into_u32());

        // Calculate ticks until alarm expires from current time
        let current = now.into_u32();
        let ticks_until_alarm = expire.wrapping_sub(now).into_u32();

        // Update current_time to reflect where we are now
        self.current_time.set(current);

        // Load the counter with the countdown value (down-counter)
        // Hardware formula: Cycle time = (PRE_8 + 1) × (CNT_32 + 1) × TCLK
        // Convert Tock ticks to hardware ticks based on prescaler compensation
        let hw_ticks_until_alarm = ticks_until_alarm * self.hw_ticks_per_tock_tick.get();

        // To get N cycles, load (N-1) into CNT_32
        // Minimum valid CNT_32 value is 1 (gives 2 cycles per hardware spec)
        let cnt_value = if hw_ticks_until_alarm >= 2 {
            hw_ticks_until_alarm - 1
        } else {
            1 // Minimum value per hardware spec (will give 2 cycles)
        };
        self.registers.cnt32.set(cnt_value);

        debug!(
            "[Itim] set_alarm: now={}, expire={}, ticks={}, hw_ticks={}, cnt_value={}, scale={}",
            current,
            expire.into_u32(),
            ticks_until_alarm,
            hw_ticks_until_alarm,
            cnt_value,
            self.hw_ticks_per_tock_tick.get()
        );

        // Enable the timer and timeout interrupt
        self.registers.cts.modify(CTS::ITEN::SET + CTS::TO_IE::SET);
    }

    fn get_alarm(&self) -> Self::Ticks {
        Self::Ticks::from(self.alarm_value.get().unwrap_or(0))
    }

    fn disarm(&self) -> Result<(), ErrorCode> {
        unsafe {
            atomic(|| {
                // Disable timeout interrupt
                self.registers.cts.modify(CTS::TO_IE::CLEAR);
                self.alarm_value.clear();
                cortexm4f::nvic::Nvic::new(self.irqn).clear_pending();
            });
        }
        Ok(())
    }

    fn is_armed(&self) -> bool {
        // Check if timeout interrupt is enabled and alarm value is set
        self.registers.cts.is_set(CTS::TO_IE) && self.alarm_value.is_some()
    }

    fn minimum_dt(&self) -> Self::Ticks {
        Self::Ticks::from(1)
    }
}

// struct ItimClock<'a>(rcc::PeripheralClock<'a>);

// impl ClockInterface for ItimClock<'_> {
//     fn is_enabled(&self) -> bool {
//         self.0.is_enabled()
//     }

//     fn enable(&self) {
//         self.0.enable();
//     }

//     fn disable(&self) {
//         self.0.disable();
//     }
// }
