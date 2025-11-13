// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Watchdog timer

use core::cell::Cell;
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, register_structs, ReadWrite};
use kernel::utilities::StaticRef;

const TWD_BASE: StaticRef<TwdRegisters> =
    unsafe { StaticRef::new(0x400D_8000 as *const TwdRegisters) };

register_structs! {
    TwdRegisters {
        (0x000 => twcfg: ReadWrite<u8, Twcfg::Register>), // Timer and Watchdog Configuration
        (0x001 => _reserved0: u8),
        (0x002 => twcp: ReadWrite<u8, Twcp::Register>), // Timer and Watchdog Clock Prescaler
        (0x003 => _reserved1: u8),
        (0x004 => twdt0: ReadWrite<u16, Twdt0::Register>), // TWD Timer 0 Counter Preset
        (0x006 => t0csr: ReadWrite<u8, T0csr::Register>), // TWDT0 Control and Status
        (0x007 => _reserved2: u8),
        (0x008 => wdcnt: ReadWrite<u8, Wdcnt::Register>), // Watchdog Count
        (0x009 => _reserved3: u8),
        (0x00A => wdsdm: ReadWrite<u8, Wdsdm::Register>), // Watchdog Service Data Match
        (0x00B => _reserved4: u8),
        (0x00C => twmt0: ReadWrite<u16, Twmt0::Register>), // TWD Timer 0 Counter
        (0x00E => twmwd: ReadWrite<u8, Twmwd::Register>), // Watchdog Counter
        (0x00F => _reserved5: u8),
        (0x010 => wdcp: ReadWrite<u8, Wdcp::Register>), // Watchdog Clock Prescaler
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

pub trait WatchdogClient {
    fn watchdog_fired(&self);
}

pub struct Wdg<'a> {
    registers: StaticRef<TwdRegisters>,
    enabled: Cell<bool>,
    client: OptionalCell<&'a dyn WatchdogClient>,
}

impl<'a> Wdg<'a> {
    pub const fn new() -> Self {
        Self {
            registers: TWD_BASE,
            enabled: Cell::new(false),
            client: OptionalCell::empty(),
        }
    }

    pub fn set_client(&self, client: &'a dyn WatchdogClient) {
        self.client.set(client);
    }

    pub fn enable(&self) {
        self.enabled.set(true);
    }

    #[allow(dead_code)]
    fn set_window(&self, _value: u32) {}

    /// Modifies the time base of the prescaler.
    #[allow(dead_code)]
    fn set_prescaler(&self, _time_base: u8) {}

    pub fn start(&self) {}

    pub fn tickle(&self) {}

    pub fn handle_interrupt(&self) {
        // This is called when the watchdog timer expires.
        self.client.map(|client| client.watchdog_fired());
        self.tickle();
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

impl<'a> kernel::platform::watchdog::WatchDog for Wdg<'a> {
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
            // self.clock.disable();
        }
    }

    fn resume(&self) {
        if self.enabled.get() {
            // self.clock.enable();
        }
    }
}
