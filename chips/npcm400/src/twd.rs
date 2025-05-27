// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Watchdog timer

use core::cell::Cell;
use kernel::platform::chip::ClockInterface;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, register_structs, ReadWrite};
use kernel::utilities::StaticRef;

const TWD_BASE: StaticRef<TwdRegisters> =
    unsafe { StaticRef::new(0x400D_8000 as *const TwdRegisters) };

register_structs! {
    TwdRegisters {
        (0x000 => twcfg: ReadWrite<u8, Twcfg::Register>), // Timer and Watchdog Configuration
        (0x002 => twcp: ReadWrite<u8, Twcp::Register>), // Timer and Watchdog Clock Prescaler
        (0x004 => twdt0: ReadWrite<u16>), // TWD Timer 0 Counter Preset
        (0x006 => t0csr: ReadWrite<u8>), // TWDT0 Control and Status
        (0x008 => wdcnt: ReadWrite<u8>), // Watchdog Count
        (0x00A => wdsdm: ReadWrite<u8>), // Watchdog Service Data Match
        (0x00C => twmt0: ReadWrite<u16>), // TWD Timer 0 Counter
        (0x00E => twmwd: ReadWrite<u8>), // Watchdog Counter
        (0x010 => wdcp: ReadWrite<u8>), // Watchdog Clock Prescaler
        (0x011 => @END),
    }
}

register_bitfields![u8,
    Twcfg [
        LTWD_CFG  OFFSET(0) NUMBITS(1) [],   // Lock TWCFG register
        LTWCP     OFFSET(1) NUMBITS(1) [],   // Lock TWCP register
        LTWDT0    OFFSET(2) NUMBITS(1) [],   // Lock TWDT0 register
        LWDCNT    OFFSET(3) NUMBITS(1) [],   // Lock WDCNT register
        WDCT0I    OFFSET(4) NUMBITS(1) [],   // Select T0IN as watchdog prescaler clock
        WDSDME    OFFSET(5) NUMBITS(1) [],   // Feed watchdog by writing 5Ch to WDSDM
        // Bits 6-7 reserved
    ],
    Twcp [
        MDIV OFFSET(0) NUMBITS(4) [] // Prescale ratio of the input clock
        // Bits 4-7 reserved
    ],
    T0csr [
        RST        OFFSET(0) NUMBITS(1) [], // Force timer0 to reload and restart
        TC         OFFSET(1) NUMBITS(1) [], // Timer0 counter reaches 0
        // Bit 2 reserved
        WDLTD      OFFSET(3) NUMBITS(1) [], // Watchdog touch performed
        WDRST_STS  OFFSET(4) NUMBITS(1) [], // Generate watchdog reset
        WD_RUN     OFFSET(5) NUMBITS(1) [], // Watchdog counter status
        T0EN       OFFSET(6) NUMBITS(1) [], // Enable t0out
        TESDIS     OFFSET(7) NUMBITS(1) [], // Disable watchdog event triggered
    ],
    Wdcnt [
        VALUE OFFSET(0) NUMBITS(8) [],      // Watchdog counter preset
    ],
    Wdsdm [
        VALUE OFFSET(0) NUMBITS(8) [],      // Watchdog restart data
    ],
    Twmwd [
        VALUE OFFSET(0) NUMBITS(8) [],      // Watchdog counter value
    ],
    Wdcp [
        VALUE OFFSET(0) NUMBITS(8) [],      // Watchdog clock prescale ratio
    ],
];

register_bitfields![u16,
    Twdt0 [
        VALUE OFFSET(0) NUMBITS(16) [], // TWD Timer 0 Counter Preset
    ],
    Twmt0 [
        VALUE OFFSET(0) NUMBITS(16) [], // Timer 0 Counter Value
    ],
];

pub struct Wdg<'a> {
    registers: StaticRef<TwdRegisters>,
    enabled: Cell<bool>,
}

impl<'a> Wdg<'a> {
    pub const fn new() -> Self {
        Self {
            registers: TWD_BASE,
            enabled: Cell::new(false),
        }
    }

    pub fn enable(&self) {
        self.enabled.set(true);
    }

    fn set_window(&self, value: u32) {
        // Set the window value to the biggest possible one.
        self.registers.cfr.modify(Config::W.val(value));
    }

    /// Modifies the time base of the prescaler.
    /// 0 - decrements the watchdog every clock cycle
    /// 1 - decrements the watchdog every 2nd clock cycle
    /// 2 - decrements the watchdog every 4th clock cycle
    /// 3 - decrements the watchdog every 8th clock cycle
    fn set_prescaler(&self, time_base: u8) {
        match time_base {
            0 => self.registers.cfr.modify(Config::WDGTB::DIVONE),
            1 => self.registers.cfr.modify(Config::WDGTB::DIVTWO),
            2 => self.registers.cfr.modify(Config::WDGTB::DIVFOUR),
            3 => self.registers.cfr.modify(Config::WDGTB::DIVEIGHT),
            _ => {}
        }
    }

    pub fn start(&self) {
        // Enable the APB1 clock for the watchdog.
        self.clock.enable();

        // This disables the window feature. Set this to a value smaller than
        // 0x7F if you want to enable it.
        self.set_window(0x7F);
        self.set_prescaler(3);

        // Set the T[6] bit to avoid a reset when the watchdog is activated.
        self.tickle();

        // With the APB1 clock running at 36Mhz we are getting timeout value of
        // t_WWDG = (1 / 36000) * 4096 * 2^3 * (63 + 1) = 58ms
        self.registers.cr.modify(Control::WDGA::SET);
    }

    pub fn tickle(&self) {
        // Uses 63 as the value the watchdog starts counting from.
        self.registers.cr.modify(Control::T.val(0x7F));
    }

    pub fn finalize(&self) {
        // Setup watchdog configs: Enable WDSDME and WDCT0I bits in TWCFG
        self.registers
            .twcfg
            .modify(Twcfg::WDSDME::SET + Twcfg::WDCT0I::SET);

        // Disable early touch functionality:
        // Clear WDRST_STS and set TESDIS in T0CSR
        let t0csr = self.registers.t0csr.get();
        self.registers.t0csr.set((t0csr & !(1 << 4)) | (1 << 7));

        // Find the power of 2 to the pre-scale ratio
        // Assuming NPCM_T0_PRESCALER and NPCM_WDT_PRESCALER are defined as constants
        // and LOG2 is replaced by .trailing_zeros() as in Rust
        const NPCM_T0_PRESCALER: u8 = 32;
        const NPCM_WDT_PRESCALER: u8 = 32;
        self.registers
            .twcp
            .modify(Twcp::MDIV.val(NPCM_T0_PRESCALER.trailing_zeros() as u8));
        self.registers
            .wdcp
            .set(NPCM_WDT_PRESCALER.trailing_zeros() as u8);
    }
}

impl kernel::platform::watchdog::WatchDog for Wdg<'_> {
    fn setup(&self) {
        if self.enabled.get() {
            self.start();
        }
    }

    fn tickle(&self) {
        if self.enabled.get() {
            self.tickle();
        }
    }

    fn suspend(&self) {
        if self.enabled.get() {
            self.clock.disable();
        }
    }

    fn resume(&self) {
        if self.enabled.get() {
            self.clock.enable();
        }
    }
}
