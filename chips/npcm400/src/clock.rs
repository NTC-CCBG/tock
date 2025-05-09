// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Clock peripheral driver, nRF52
//!
//! Based on Phil Levis clock driver for nRF51
//!
//! HFCLK - High Frequency Clock:
//!
//! * 64 MHz internal oscillator (HFINT)
//! * 64 MHz crystal oscillator, using 32 MHz external crystal (HFXO)
//! * The HFXO must be running to use the RADIO, NFC module or the calibration mechanism
//!   associated with the 32.768 kHz RC oscillator.
//!
//! LFCLK - Low Frequency Clock Source:
//!
//! * 32.768 kHz RC oscillator (LFRC)
//! * 32.768 kHz crystal oscillator (LFXO)
//! * 32.768 kHz synthesized from HFCLK (LFSYNT)
//!

use kernel::utilities::cells::OptionalCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{
    register_bitfields, register_structs, ReadWrite,
};
use kernel::utilities::StaticRef;

register_structs! {
    CdcgRegisters {
        (0x000 => hfcgctrl: ReadWrite<u8, HfcgCtrl::Register>),
        (0x001 => _reserved1),
        (0x002 => hfcgml: ReadWrite<u8, HfcgMl::Register>),
        (0x003 => _reserved2),
        (0x004 => hfcgmh: ReadWrite<u8, HfcgMh::Register>),
        (0x005 => _reserved3),
        (0x006 => hfcgn: ReadWrite<u8, HfcgN::Register>),
        (0x007 => _reserved4),
        (0x008 => hfcgp: ReadWrite<u8, HfcgP::Register>),
        (0x009 => _reserved5: [u8; 7]),
        (0x010 => hfcbcd: ReadWrite<u8, Hfcbcd::Register>),
        (0x011 => _reserved6),
        (0x012 => hfcbcd1: ReadWrite<u8, Hfcbcd1::Register>),
        (0x013 => _reserved7),
        (0x014 => hfcbcd2: ReadWrite<u8, Hfcbcd2::Register>),
        (0x015 => @END),
    }
}

register_bitfields! [u8,
    HfcgCtrl [
        LOAD OFFSET(0) NUMBITS(1) [],
        LOCK OFFSET(2) NUMBITS(1) [],
        CLK_CHNG OFFSET(7) NUMBITS(1) []
    ],
    HfcgMl [
        HFCGM OFFSET(0) NUMBITS(8) []
    ],
    HfcgMh [
        HFCGM OFFSET(0) NUMBITS(8) []
    ],
    HfcgN [
        HFCGN OFFSET(0) NUMBITS(6) [],
        XF_RANGE OFFSET(7) NUMBITS(1) []
    ],
    HfcgP [
        AHB6DIV OFFSET(0) NUMBITS(2) [],
        FPRED OFFSET(4) NUMBITS(4) []
    ],
    Hfcbcd [
        APB1DIV OFFSET(0) NUMBITS(4) [],
        APB2DIV OFFSET(4) NUMBITS(4) []
    ],
    Hfcbcd1 [
        FIUDIV OFFSET(0) NUMBITS(2) [],
        I3CDIV OFFSET(2) NUMBITS(2) []
    ],
    Hfcbcd2 [
        APB3DIV OFFSET(0) NUMBITS(4) []
    ]
];

const CLOCK_BASE: StaticRef<CdcgRegisters> =
    unsafe { StaticRef::new(0x400B_5000 as *const CdcgRegisters) };

register_structs! {
    PowerRegisters {
        (0x00 => pmcsr: ReadWrite<u8>),           // Power Management Controller Status Byte
        (0x01 => _reserved1),
        (0x03 => enslp_ctl: ReadWrite<u8>),       // Enable in Sleep Control Byte
        (0x04 => disidl_ctl: ReadWrite<u8>),      // Disable in Idle Control Byte
        (0x05 => disidl_ctl1: ReadWrite<u8>),     // Disable in Idle Control 1 Byte
        (0x06 => _reserved2),
        (0x07 => pwdwn_ctl0: ReadWrite<u8>),      // Power-Down Control 0 Byte
        (0x08 => pwdwn_ctl1: ReadWrite<u8, Pwdwn_ctl1::Register>),      // Power-Down Control 1 Byte
        (0x09 => pwdwn_ctl2: ReadWrite<u8>),      // Power-Down Control 2 Byte
        (0x0A => pwdwn_ctl3: ReadWrite<u8>),      // Power-Down Control 3 Byte
        (0x0B => pwdwn_ctl4: ReadWrite<u8, Pwdwn_ctl4::Register>),      // Power-Down Control 4 Byte
        (0x0C => pwdwn_ctl5: ReadWrite<u8>),      // Power-Down Control 5 Byte
        (0x0D => pwdwn_ctl6: ReadWrite<u8>),      // Power-Down Control 6 Byte
        (0x0E => _reserved3),
        (0x11 => ram_pd1: ReadWrite<u8>),         // RAM Power-Down Control 1 Byte
        (0x12 => ram_pd2: ReadWrite<u8>),         // RAM Power-Down Control 2 Byte
        (0x13 => sw_rst1: ReadWrite<u8>),         // Software Reset 1 Byte
        (0x14 => ram_pd3: ReadWrite<u8>),         // RAM Power-Down Control 3 Byte
        (0x15 => pwdwn_ctl7: ReadWrite<u8>),      // Power-Down Control 7 Byte
        (0x16 => pwdwn_ctl8: ReadWrite<u8>),      // Power-Down Control 8 Byte
        (0x17 => @END),
    }
}

register_bitfields! [u8,
    Pwdwn_ctl1 [
        FIU_PD OFFSET(2) NUMBITS(1) [],
        UART_PD OFFSET(4) NUMBITS(1) [],
    ],
    Pwdwn_ctl4 [
        ADC_PD OFFSET(4) NUMBITS(1) [],
    ],
];

const POWER_BASE: StaticRef<PowerRegisters> =
    unsafe { StaticRef::new(0x4000_D000 as *const PowerRegisters) };

/// Frequency multipliers for the different clock frequencies
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum SourceFrequency {
    _100M = 100_000_000,
    _96M = 96_000_000,
    _80M = 80_000_000,
    _66M = 66_000_000,
    _50M = 50_000_000,
    _48M = 48_000_000,
    _40M = 40_000_000,
    _33M = 33_000_000,
}

#[derive(Copy, Clone)]
pub struct FreqMultiplier {
    pub ofmclk: SourceFrequency,
    pub hfcgn: u8,
    pub hfcgmh: u8,
    pub hfcgml: u8,
}


/// Frequency multipliers for npcm400
pub static FREQ_MULTIPLIERS: [FreqMultiplier; 8] = [
    FreqMultiplier {
        ofmclk: SourceFrequency::_100M,
        hfcgn: 0x82,
        hfcgmh: 0x0B,
        hfcgml: 0xEC,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_96M,
        hfcgn: 0x82,
        hfcgmh: 0x0B,
        hfcgml: 0x72,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_80M,
        hfcgn: 0x82,
        hfcgmh: 0x09,
        hfcgml: 0x89,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_66M,
        hfcgn: 0x82,
        hfcgmh: 0x07,
        hfcgml: 0xDE,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_50M,
        hfcgn: 0x02,
        hfcgmh: 0x0B,
        hfcgml: 0xEC,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_48M,
        hfcgn: 0x02,
        hfcgmh: 0x0B,
        hfcgml: 0x72,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_40M,
        hfcgn: 0x02,
        hfcgmh: 0x09,
        hfcgml: 0x89,
    },
    FreqMultiplier {
        ofmclk: SourceFrequency::_33M,
        hfcgn: 0x02,
        hfcgmh: 0x07,
        hfcgml: 0xDE,
    },
];

/// Prescaler values for the different clock frequencies
#[derive(Copy, Clone)]
pub struct Prescaler {
    pub core: u8,
    pub apb1: u8,
    pub apb2: u8,
    pub apb3: u8,
    pub ahb6: u8,
    pub fiu: u8,
    pub i3c: u8,
}

/// Prescaler values for npcm400
pub static PRESCALER: Prescaler = Prescaler {
    core: 1,
    apb1: 8,
    apb2: 1,
    apb3: 1,
    ahb6: 1,
    fiu: 1,
    i3c: 1,
};

/// High clocks
#[derive(Copy, Clone, PartialEq)]
pub enum HighClocks {
    UART = 0,
    FIU,
    ADC,
}

/// High frequency clock source
#[derive(Copy, Clone)]
pub enum HighClockSource {
    LFCLK = 0,
    OSC,
    FIU,
    I3C,
    CORE,
    APB1,
    APB2,
    APB3,
    AHB6,
    FMCLK,
    USB20,
    SIO,
}

#[derive(Copy, Clone)]
pub struct Clocks {
    clock: HighClocks,
    source: HighClockSource,
    supported: bool,
}

impl Clocks {
    fn check_support(&self) -> bool {
        self.supported
    }

    fn clock_on(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        if !self.check_support() {
            return Err("Clock not supported");
        }
        match self.clock {
            HighClocks::UART => {
                // Clear UART_PD bit in pwdwn_ctl1
                power_reg.pwdwn_ctl1.modify(Pwdwn_ctl1::UART_PD::CLEAR);
            }
            HighClocks::FIU => {
                // Clear FIU_PD bit in pwdwn_ctl1
                power_reg.pwdwn_ctl1.modify(Pwdwn_ctl1::FIU_PD::CLEAR);
            }
            HighClocks::ADC => {
                // Clear ADC_PD bit in pwdwn_ctl4
                power_reg.pwdwn_ctl4.modify(Pwdwn_ctl4::ADC_PD::CLEAR);
            }
        }
        Ok(())
    }

    fn clock_off(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        if !self.check_support() {
            return Err("Clock not supported");
        }
        match self.clock {
            HighClocks::UART => {
                // Clear UART_PD bit in pwdwn_ctl1
                power_reg.pwdwn_ctl1.modify(Pwdwn_ctl1::UART_PD::SET);
            }
            HighClocks::FIU => {
                // Clear FIU_PD bit in pwdwn_ctl1
                power_reg.pwdwn_ctl1.modify(Pwdwn_ctl1::FIU_PD::SET);
            }
            HighClocks::ADC => {
                // Clear ADC_PD bit in pwdwn_ctl4
                power_reg.pwdwn_ctl4.modify(Pwdwn_ctl4::ADC_PD::SET);
            }
        }
        Ok(())
    }

    fn get_source(&self) -> HighClockSource {
        self.source
    }
}

pub static CLOCK_CONFIG: [Clocks; 3] = [
    Clocks {
        clock: HighClocks::UART,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::FIU,
        source: HighClockSource::FIU,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ADC,
        source: HighClockSource::APB1,
        supported: true,
    },
];

/// Clock struct
pub struct Clock {
    registers: StaticRef<CdcgRegisters>,
    registers_power: StaticRef<PowerRegisters>,
    client: OptionalCell<&'static dyn ClockClient>,
}

pub trait ClockClient {
    /// All clock interrupts are control signals, e.g., when
    /// a clock has started etc. We don't actually handle any
    /// of them for now, but keep this trait in place for if we
    /// do need to in the future.
    fn event(&self);
}

impl Clock {
    /// Constructor
    pub const fn new() -> Clock {
        Clock {
            registers: CLOCK_BASE,
            registers_power: POWER_BASE,
            client: OptionalCell::empty(),
        }
    }

    /// Client for callbacks
    pub fn set_client(&self, client: &'static dyn ClockClient) {
        self.client.set(client);
    }

    fn get_frequency_multiplier(&self, freq: SourceFrequency) -> Option<FreqMultiplier> {
        let mut freq_multiplier = None;
        for i in 0..FREQ_MULTIPLIERS.len() {
            if FREQ_MULTIPLIERS[i].ofmclk == freq {
                freq_multiplier = Some(FREQ_MULTIPLIERS[i]);
                break;
            }
        }
        freq_multiplier
    }

    fn set_frequency(&self, freq: SourceFrequency) {
        if !FREQ_MULTIPLIERS.iter().any(|fm| fm.ofmclk == freq) {
            panic!("Invalid frequency: {:?}.", freq);
        }
        let freq_multiplier = self.get_frequency_multiplier(freq);
        if let Some(freq_multiplier) = freq_multiplier {
            if freq_multiplier.hfcgn != self.registers.hfcgn.get()
                || freq_multiplier.hfcgmh != self.registers.hfcgmh.get()
                || freq_multiplier.hfcgml != self.registers.hfcgml.get()
            {
                self.registers.hfcgml.set(freq_multiplier.hfcgml);
                self.registers.hfcgmh.set(freq_multiplier.hfcgmh);
                self.registers.hfcgn.set(freq_multiplier.hfcgn);
            }
        } else {
            // Log or handle the error case where freq_multiplier is None
            panic!("Frequency multiplier not found for frequency: {:?}", freq);
        }
    }

    fn get_prescaler(&self) -> Prescaler {
        let prescaler = PRESCALER;
        prescaler
    }

    fn set_prescaler(&self) {
        let prescaler = self.get_prescaler();
        self.registers
            .hfcgp
            .modify(HfcgP::FPRED.val(prescaler.fiu) + HfcgP::AHB6DIV.val(prescaler.ahb6));
        self.registers
            .hfcbcd
            .modify(Hfcbcd::APB1DIV.val(prescaler.apb1) + Hfcbcd::APB2DIV.val(prescaler.apb2));
        self.registers
            .hfcbcd1
            .modify(Hfcbcd1::FIUDIV.val(prescaler.fiu));
        self.registers
            .hfcbcd2
            .modify(Hfcbcd2::APB3DIV.val(prescaler.apb3));
    }

    pub fn config_clock(&self, freq: SourceFrequency) {
        self.set_frequency(freq);
        self.set_prescaler();
    }

    fn find_clock_config(&self, clock: HighClocks) -> Option<&'static Clocks> {
        for i in 0..CLOCK_CONFIG.len() {
            if CLOCK_CONFIG[i].clock == clock {
                return Some(&CLOCK_CONFIG[i]);
            }
        }
        None
    }

    pub fn high_clock_on(&self, clock: HighClocks) {
        if let Some(clock_config) = self.find_clock_config(clock) {
            if let Err(e) = clock_config.clock_on(self.registers_power) {
                // Handle the error, e.g., log it or propagate it
                panic!("Failed to turn on high clock: {}", e);
            }
        }
    }

    pub fn high_clock_off(&self, clock: HighClocks) {
        if let Some(clock_config) = self.find_clock_config(clock) {
            if let Err(e) = clock_config.clock_off(self.registers_power) {
                // Handle the error, e.g., log it or propagate it
                panic!("Failed to turn off high clock: {}", e);
            }
        }
    }

    pub fn get_clock_source(&self, clock: HighClocks) -> Option<HighClockSource> {
        let clock_config = self.find_clock_config(clock);
        clock_config.map(|config| config.get_source())
    }
}
