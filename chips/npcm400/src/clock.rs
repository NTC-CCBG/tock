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
use kernel::utilities::registers::{register_bitfields, register_structs, ReadWrite};
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
        (0x07 => pwdwn_ctl0: ReadWrite<u8, Pwdwn_ctl0::Register>),      // Power-Down Control 0 Byte
        (0x08 => pwdwn_ctl1: ReadWrite<u8, Pwdwn_ctl1::Register>),      // Power-Down Control 1 Byte
        (0x09 => pwdwn_ctl2: ReadWrite<u8, Pwdwn_ctl2::Register>),      // Power-Down Control 2 Byte
        (0x0A => pwdwn_ctl3: ReadWrite<u8, Pwdwn_ctl3::Register>),      // Power-Down Control 3 Byte
        (0x0B => pwdwn_ctl4: ReadWrite<u8, Pwdwn_ctl4::Register>),      // Power-Down Control 4 Byte
        (0x0C => pwdwn_ctl5: ReadWrite<u8, Pwdwn_ctl5::Register>),      // Power-Down Control 5 Byte
        (0x0D => pwdwn_ctl6: ReadWrite<u8, Pwdwn_ctl6::Register>),      // Power-Down Control 6 Byte
        (0x0E => _reserved3),
        (0x11 => ram_pd1: ReadWrite<u8>),         // RAM Power-Down Control 1 Byte
        (0x12 => ram_pd2: ReadWrite<u8>),         // RAM Power-Down Control 2 Byte
        (0x13 => sw_rst1: ReadWrite<u8>),         // Software Reset 1 Byte
        (0x14 => ram_pd3: ReadWrite<u8>),         // RAM Power-Down Control 3 Byte
        (0x15 => pwdwn_ctl7: ReadWrite<u8, Pwdwn_ctl7::Register>),      // Power-Down Control 7 Byte
        (0x16 => pwdwn_ctl8: ReadWrite<u8, Pwdwn_ctl8::Register>),      // Power-Down Control 8 Byte
        (0x17 => @END),
    }
}

register_bitfields! [u8,
    Pwdwn_ctl0 [
        PWM_I_PD OFFSET(0) NUMBITS(1) [],
        PWM_J_PD OFFSET(1) NUMBITS(1) [],
        I3CI_PD OFFSET(2) NUMBITS(1) [],
        UART3_PD OFFSET(5) NUMBITS(1) [],
        UART2_PD OFFSET(6) NUMBITS(1) [],
    ],
    Pwdwn_ctl1 [
        SPIM_PD OFFSET(0) NUMBITS(1) [],
        FIU_PD OFFSET(2) NUMBITS(1) [],
        USB20_PD OFFSET(3) NUMBITS(1) [],
        UART_PD OFFSET(4) NUMBITS(1) [],
        MFT1_PD OFFSET(5) NUMBITS(1) [],
        MFT2_PD OFFSET(6) NUMBITS(1) [],
        MFT3_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl2 [
        PWM_A_PD OFFSET(0) NUMBITS(1) [],
        PWM_B_PD OFFSET(1) NUMBITS(1) [],
        PWM_C_PD OFFSET(2) NUMBITS(1) [],
        PWM_D_PD OFFSET(3) NUMBITS(1) [],
        PWM_E_PD OFFSET(4) NUMBITS(1) [],
        PWM_F_PD OFFSET(5) NUMBITS(1) [],
        PWM_G_PD OFFSET(6) NUMBITS(1) [],
        PWM_H_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl3 [
        GDMA_PD OFFSET(0) NUMBITS(1) [],
        SMB6_PD OFFSET(1) NUMBITS(1) [],
        SMB5_PD OFFSET(2) NUMBITS(1) [],
        SMB4_PD OFFSET(3) NUMBITS(1) [],
        SMB3_PD OFFSET(4) NUMBITS(1) [],
        SMB2_PD OFFSET(5) NUMBITS(1) [],
        SMB1_PD OFFSET(6) NUMBITS(1) [],
    ],
    Pwdwn_ctl4 [
        ITIM1_PD OFFSET(0) NUMBITS(1) [],
        ITIM2_PD OFFSET(1) NUMBITS(1) [],
        ITIM3_PD OFFSET(2) NUMBITS(1) [],
        SMB_DMA_PD OFFSET(3) NUMBITS(1) [],
        ADC_PD OFFSET(4) NUMBITS(1) [],
        PECI_PD OFFSET(5) NUMBITS(1) [],
        SPIP1_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl5 [
        UART4_PD OFFSET(0) NUMBITS(1) [],
        C2HACC_PD OFFSET(3) NUMBITS(1) [],
        SHM_REG_PD OFFSET(4) NUMBITS(1) [],
        SHM_PD OFFSET(5) NUMBITS(1) [],
        DP80_PD OFFSET(6) NUMBITS(1) [],
        MSWC_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl6 [
        ITIM4_PD OFFSET(0) NUMBITS(1) [],
        ITIM5_PD OFFSET(1) NUMBITS(1) [],
        ITIM6_PD OFFSET(2) NUMBITS(1) [],
        RNG_PD OFFSET(3) NUMBITS(1) [],
        SHA_PD OFFSET(5) NUMBITS(1) [],
        eSPI_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl7 [
        SMB7_PD OFFSET(0) NUMBITS(1) [],
        SMB8_PD OFFSET(1) NUMBITS(1) [],
        SMB9_PD OFFSET(2) NUMBITS(1) [],
        SMB10_PD OFFSET(3) NUMBITS(1) [],
        SMB11_PD OFFSET(4) NUMBITS(1) [],
        SMB12_PD OFFSET(5) NUMBITS(1) [],
        SIOX2_PD OFFSET(6) NUMBITS(1) [],
        SIOX1_PD OFFSET(7) NUMBITS(1) [],
    ],
    Pwdwn_ctl8 [
        I3CI2_PD OFFSET(0) NUMBITS(1) [],
        I3CI3_PD OFFSET(1) NUMBITS(1) [],
        I3CI4_PD OFFSET(2) NUMBITS(1) [],
        I3CI5_PD OFFSET(3) NUMBITS(1) [],
        I3CI6_PD OFFSET(4) NUMBITS(1) [],
    ]
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
#[allow(non_camel_case_types)]
pub enum HighClocks {
    PWM_I = 0,
    PWM_J,
    I3CI,
    UART3,
    UART2,
    SPIM,
    FIU,
    USB20,
    UART,
    MFT1,
    MFT2,
    MFT3,
    PWM_A,
    PWM_B,
    PWM_C,
    PWM_D,
    PWM_E,
    PWM_F,
    PWM_G,
    PWM_H,
    SMB1,
    SMB2,
    SMB3,
    SMB4,
    SMB5,
    SMB6,
    GDMA,
    ITIM1,
    ITIM2,
    ITIM3,
    SMB_DMA,
    ADC,
    PECI,
    SPIP1,
    UART4,
    C2HACC,
    SHM_REG,
    SHM,
    DP80,
    MSWC,
    ITIM4,
    ITIM5,
    ITIM6,
    RNG,
    SHA,
    ESPI,
    SMB7,
    SMB8,
    SMB9,
    SMB10,
    SMB11,
    SMB12,
    SIOX2,
    SIOX1,
    I3CI2,
    I3CI3,
    I3CI4,
    I3CI5,
    I3CI6,
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

// Power down register mapping
enum PowerDownReg {
    CTL0,
    CTL1,
    CTL2,
    CTL3,
    CTL4,
    CTL5,
    CTL6,
    CTL7,
    CTL8,
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

    // Helper: map HighClocks to (register, field)
    fn get_pd_reg_field(&self, clock: HighClocks) -> Option<(PowerDownReg, u8)> {
        Some(match clock {
            HighClocks::PWM_I => (PowerDownReg::CTL0, 0),
            HighClocks::PWM_J => (PowerDownReg::CTL0, 1),
            HighClocks::I3CI => (PowerDownReg::CTL0, 2),
            HighClocks::UART3 => (PowerDownReg::CTL0, 5),
            HighClocks::UART2 => (PowerDownReg::CTL0, 6),
            HighClocks::SPIM => (PowerDownReg::CTL1, 0),
            HighClocks::FIU => (PowerDownReg::CTL1, 2),
            HighClocks::USB20 => (PowerDownReg::CTL1, 3),
            HighClocks::UART => (PowerDownReg::CTL1, 4),
            HighClocks::MFT1 => (PowerDownReg::CTL1, 5),
            HighClocks::MFT2 => (PowerDownReg::CTL1, 6),
            HighClocks::MFT3 => (PowerDownReg::CTL1, 7),
            HighClocks::PWM_A => (PowerDownReg::CTL2, 0),
            HighClocks::PWM_B => (PowerDownReg::CTL2, 1),
            HighClocks::PWM_C => (PowerDownReg::CTL2, 2),
            HighClocks::PWM_D => (PowerDownReg::CTL2, 3),
            HighClocks::PWM_E => (PowerDownReg::CTL2, 4),
            HighClocks::PWM_F => (PowerDownReg::CTL2, 5),
            HighClocks::PWM_G => (PowerDownReg::CTL2, 6),
            HighClocks::PWM_H => (PowerDownReg::CTL2, 7),
            HighClocks::GDMA => (PowerDownReg::CTL3, 0),
            HighClocks::SMB6 => (PowerDownReg::CTL3, 1),
            HighClocks::SMB5 => (PowerDownReg::CTL3, 2),
            HighClocks::SMB4 => (PowerDownReg::CTL3, 3),
            HighClocks::SMB3 => (PowerDownReg::CTL3, 4),
            HighClocks::SMB2 => (PowerDownReg::CTL3, 5),
            HighClocks::SMB1 => (PowerDownReg::CTL3, 6),
            HighClocks::ITIM1 => (PowerDownReg::CTL4, 0),
            HighClocks::ITIM2 => (PowerDownReg::CTL4, 1),
            HighClocks::ITIM3 => (PowerDownReg::CTL4, 2),
            HighClocks::SMB_DMA => (PowerDownReg::CTL4, 3),
            HighClocks::ADC => (PowerDownReg::CTL4, 4),
            HighClocks::PECI => (PowerDownReg::CTL4, 5),
            HighClocks::SPIP1 => (PowerDownReg::CTL4, 7),
            HighClocks::UART4 => (PowerDownReg::CTL5, 0),
            HighClocks::C2HACC => (PowerDownReg::CTL5, 3),
            HighClocks::SHM_REG => (PowerDownReg::CTL5, 4),
            HighClocks::SHM => (PowerDownReg::CTL5, 5),
            HighClocks::DP80 => (PowerDownReg::CTL5, 6),
            HighClocks::MSWC => (PowerDownReg::CTL5, 7),
            HighClocks::ITIM4 => (PowerDownReg::CTL6, 0),
            HighClocks::ITIM5 => (PowerDownReg::CTL6, 1),
            HighClocks::ITIM6 => (PowerDownReg::CTL6, 2),
            HighClocks::RNG => (PowerDownReg::CTL6, 3),
            HighClocks::SHA => (PowerDownReg::CTL6, 5),
            HighClocks::ESPI => (PowerDownReg::CTL6, 7),
            HighClocks::SMB7 => (PowerDownReg::CTL7, 0),
            HighClocks::SMB8 => (PowerDownReg::CTL7, 1),
            HighClocks::SMB9 => (PowerDownReg::CTL7, 2),
            HighClocks::SMB10 => (PowerDownReg::CTL7, 3),
            HighClocks::SMB11 => (PowerDownReg::CTL7, 4),
            HighClocks::SMB12 => (PowerDownReg::CTL7, 5),
            HighClocks::SIOX2 => (PowerDownReg::CTL7, 6),
            HighClocks::SIOX1 => (PowerDownReg::CTL7, 7),
            HighClocks::I3CI2 => (PowerDownReg::CTL8, 0),
            HighClocks::I3CI3 => (PowerDownReg::CTL8, 1),
            HighClocks::I3CI4 => (PowerDownReg::CTL8, 2),
            HighClocks::I3CI5 => (PowerDownReg::CTL8, 3),
            HighClocks::I3CI6 => (PowerDownReg::CTL8, 4),
        })
    }

    fn set_pd_bit(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        if !self.check_support() {
            return Err("Clock not supported");
        }

        if let Some((reg, bit)) = self.get_pd_reg_field(self.clock) {
            let mask = 1 << bit;
            match reg {
                PowerDownReg::CTL0 => power_reg.pwdwn_ctl0.set(power_reg.pwdwn_ctl0.get() | mask),
                PowerDownReg::CTL1 => power_reg.pwdwn_ctl1.set(power_reg.pwdwn_ctl1.get() | mask),
                PowerDownReg::CTL2 => power_reg.pwdwn_ctl2.set(power_reg.pwdwn_ctl2.get() | mask),
                PowerDownReg::CTL3 => power_reg.pwdwn_ctl3.set(power_reg.pwdwn_ctl3.get() | mask),
                PowerDownReg::CTL4 => power_reg.pwdwn_ctl4.set(power_reg.pwdwn_ctl4.get() | mask),
                PowerDownReg::CTL5 => power_reg.pwdwn_ctl5.set(power_reg.pwdwn_ctl5.get() | mask),
                PowerDownReg::CTL6 => power_reg.pwdwn_ctl6.set(power_reg.pwdwn_ctl6.get() | mask),
                PowerDownReg::CTL7 => power_reg.pwdwn_ctl7.set(power_reg.pwdwn_ctl7.get() | mask),
                PowerDownReg::CTL8 => power_reg.pwdwn_ctl8.set(power_reg.pwdwn_ctl8.get() | mask),
            }
            Ok(())
        } else {
            Err("Clock not supported")
        }
    }

    fn clear_pd_bit(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        if !self.check_support() {
            return Err("Clock not supported");
        }

        if let Some((reg, bit)) = self.get_pd_reg_field(self.clock) {
            let mask = 1 << bit;
            match reg {
                PowerDownReg::CTL0 => power_reg.pwdwn_ctl0.set(power_reg.pwdwn_ctl0.get() & !mask),
                PowerDownReg::CTL1 => power_reg.pwdwn_ctl1.set(power_reg.pwdwn_ctl1.get() & !mask),
                PowerDownReg::CTL2 => power_reg.pwdwn_ctl2.set(power_reg.pwdwn_ctl2.get() & !mask),
                PowerDownReg::CTL3 => power_reg.pwdwn_ctl3.set(power_reg.pwdwn_ctl3.get() & !mask),
                PowerDownReg::CTL4 => power_reg.pwdwn_ctl4.set(power_reg.pwdwn_ctl4.get() & !mask),
                PowerDownReg::CTL5 => power_reg.pwdwn_ctl5.set(power_reg.pwdwn_ctl5.get() & !mask),
                PowerDownReg::CTL6 => power_reg.pwdwn_ctl6.set(power_reg.pwdwn_ctl6.get() & !mask),
                PowerDownReg::CTL7 => power_reg.pwdwn_ctl7.set(power_reg.pwdwn_ctl7.get() & !mask),
                PowerDownReg::CTL8 => power_reg.pwdwn_ctl8.set(power_reg.pwdwn_ctl8.get() & !mask),
            }
            Ok(())
        } else {
            Err("Clock not supported")
        }
    }

    fn clock_on(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        self.clear_pd_bit(power_reg)
    }

    fn clock_off(&self, power_reg: StaticRef<PowerRegisters>) -> Result<(), &str> {
        self.set_pd_bit(power_reg)
    }

    #[allow(dead_code)]
    fn get_source(&self) -> HighClockSource {
        self.source
    }
}

pub static CLOCK_CONFIG: [Clocks; 9] = [
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
    Clocks {
        clock: HighClocks::ITIM1,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ITIM2,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ITIM3,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ITIM4,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ITIM5,
        source: HighClockSource::APB2,
        supported: true,
    },
    Clocks {
        clock: HighClocks::ITIM6,
        source: HighClockSource::APB2,
        supported: true,
    },
];

/// Clock struct
pub struct Clock {
    registers: StaticRef<CdcgRegisters>,
    registers_power: StaticRef<PowerRegisters>,
    client: OptionalCell<&'static dyn ClockClient>,
    pub source_freq: SourceFrequency,
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
    pub const fn new(src: SourceFrequency) -> Clock {
        Clock {
            registers: CLOCK_BASE,
            registers_power: POWER_BASE,
            client: OptionalCell::empty(),
            source_freq: src,
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

        // Resetting the OFMCLK (even to the same value) will make the clock
        // unstable for a little which can affect peripheral communication like
        // eSPI. Skip this if not needed.
        if let Some(freq_multiplier) = freq_multiplier {
            if freq_multiplier.hfcgn != self.registers.hfcgn.get()
                || freq_multiplier.hfcgmh != self.registers.hfcgmh.get()
                || freq_multiplier.hfcgml != self.registers.hfcgml.get()
            {
                // Configure frequency multiplier M/N values according to
                // the requested OFMCLK (Unit:Hz).
                self.registers.hfcgml.set(freq_multiplier.hfcgml);
                self.registers.hfcgmh.set(freq_multiplier.hfcgmh);
                self.registers.hfcgn.set(freq_multiplier.hfcgn);

                // Load M and N values into the frequency multiplier
                self.registers.hfcgctrl.modify(HfcgCtrl::LOAD::SET);

                // Wait for stable
                while self.registers.hfcgctrl.is_set(HfcgCtrl::CLK_CHNG) {}
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

    // Set all clock prescalers of core and peripherals.
    fn set_prescaler(&self) {
        let prescaler = self.get_prescaler();

        self.registers.hfcgp.set(prescaler.fiu + prescaler.ahb6);
        self.registers.hfcbcd.set(prescaler.apb1 + prescaler.apb2);
        self.registers.hfcbcd1.set(prescaler.fiu);
        self.registers.hfcbcd2.set(prescaler.apb3);
    }

    pub fn config_clock(&self) {
        self.set_frequency(self.source_freq);
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
            clock_config.clock_on(self.registers_power).unwrap()
        }
    }

    pub fn high_clock_off(&self, clock: HighClocks) {
        if let Some(clock_config) = self.find_clock_config(clock) {
            clock_config.clock_off(self.registers_power).unwrap()
        }
    }

    pub fn get_clock_source(&self, clock: HighClocks) -> Option<u32> {
        let clock_config = self.find_clock_config(clock)?;
        let prescaler = self.get_prescaler();
        Some(match clock_config.source {
            HighClockSource::APB1 => self.source_freq as u32 / prescaler.apb1 as u32,
            HighClockSource::APB2 => self.source_freq as u32 / prescaler.apb2 as u32,
            HighClockSource::APB3 => self.source_freq as u32 / prescaler.apb3 as u32,
            HighClockSource::AHB6 => self.source_freq as u32 / prescaler.ahb6 as u32,
            HighClockSource::FIU => self.source_freq as u32 / prescaler.fiu as u32,
            HighClockSource::I3C => {
                (self.source_freq as u32 / prescaler.apb1 as u32) / prescaler.i3c as u32
            }
            HighClockSource::CORE => self.source_freq as u32 / prescaler.core as u32,
            HighClockSource::OSC => self.source_freq as u32,
            HighClockSource::LFCLK => 32_768_u32,
            HighClockSource::FMCLK => {
                if self.source_freq as u32 > 50_000_000_u32 {
                    self.source_freq as u32 / 2
                } else {
                    self.source_freq as u32
                }
            }
            HighClockSource::USB20 => 12_000_000_u32,
            HighClockSource::SIO => 48_000_000_u32,
        })
    }
}
