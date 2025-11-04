// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! SCFG

use kernel::utilities::registers::interfaces::ReadWriteable;
use kernel::utilities::registers::{
    register_bitfields, register_structs, ReadWrite
};
use kernel::utilities::StaticRef;

register_structs! {
    ScfgRegisters {
        (0x000 => _reserved: [u8; 11]),
        (0x00B => devalt10: ReadWrite<u8, Devalt10::Register>),
        (0x00C => _reserved1: [u8; 9]),
        (0x015 => devalt5: ReadWrite<u8, Devalt5::Register>),
        (0x016 => _reserved0: [u8; 1]),
        (0x017 => devalt7: ReadWrite<u8, Devalt7::Register>),
        (0x018 => _reserved2: [u8; 2]),
        (0x01A => devalta: ReadWrite<u8, Devalta::Register>),
        (0x01B => _reserved3: [u8; 1]),
        (0x01C => devaltc: ReadWrite<u8, Devaltc::Register>),
        (0x01D => _reserved4: [u8; 12]),
        (0x029 => devpd1: ReadWrite<u8, Devpd1::Register>),
        (0x02A => _reserved5: [u8; 81]),
        (0x07B => devpd3: ReadWrite<u8, Devpd3::Register>),
        (0x07C => @END),
    }
}

register_bitfields! [u8,
    Devalt10 [
        /// I3C1 Select (bit 6)
        I3C1_SL OFFSET(6) NUMBITS(1) [],
        /// I3C4 Select (bit 3)
        I3C4_SL OFFSET(3) NUMBITS(1) [],
        /// I3C3 Select (bit 2)
        I3C3_SL OFFSET(2) NUMBITS(1) [],
        /// I3C2 Select (bit 1)
        I3C2_SL OFFSET(1) NUMBITS(1) []
    ],
    Devalt5 [
        PECI_EN OFFSET(4) NUMBITS(1) []
    ],
    Devalt7 [
        /// I3C6 Select (bit 2)
        GP97_SL OFFSET(4) NUMBITS(1) [],
        GP96_SL OFFSET(3) NUMBITS(1) [],
        I3C6_SL OFFSET(2) NUMBITS(1) []
    ],
    Devalta [
        URTO2_SL OFFSET(7) NUMBITS(1) [],
        /// I3C5 Select (bit 4)
        I3C5_SL OFFSET(4) NUMBITS(1) []
    ],
    Devaltc [
        URTI1_SL OFFSET(6) NUMBITS(1) []
    ],
    Devpd1 [
        /// I3C1 Pull-Up Enable (bit 2)
        I3C1_PUE OFFSET(2) NUMBITS(1) []
    ],
    Devpd3 [
        /// I3C6 Pull-Up Enable (bit 4)
        I3C6_PUE OFFSET(4) NUMBITS(1) [],
        /// I3C5 Pull-Up Enable (bit 3)
        I3C5_PUE OFFSET(3) NUMBITS(1) [],
        /// I3C4 Pull-Up Enable (bit 2)
        I3C4_PUE OFFSET(2) NUMBITS(1) [],
        /// I3C3 Pull-Up Enable (bit 1)
        I3C3_PUE OFFSET(1) NUMBITS(1) [],
        /// I3C2 Pull-Up Enable (bit 0)
        I3C2_PUE OFFSET(0) NUMBITS(1) []
    ],
];

const SCFG_BASE: StaticRef<ScfgRegisters> =
    unsafe { StaticRef::new(0x400C_3000 as *const ScfgRegisters) };

pub struct Scfg {
    registers: StaticRef<ScfgRegisters>,
}

pub enum ScfgField {
    Urto2Sl,
    Urti1Sl,
    I3c1Sl,
    I3c2Sl,
    I3c3Sl,
    I3c4Sl,
    I3c5Sl,
    I3c6Sl,
}

impl Scfg {
    /// Constructor
    pub const fn new() -> Scfg {
        Scfg {
            registers: SCFG_BASE,
        }
    }

    pub fn init(&self) {
        self.registers.devalta.modify(Devalta::URTO2_SL::SET);
        self.registers.devaltc.modify(Devaltc::URTI1_SL::SET);
    }

    pub fn set_field(&self, field: ScfgField) {
        match field {
            ScfgField::Urto2Sl => self.registers.devalta.modify(Devalta::URTO2_SL::SET),
            ScfgField::Urti1Sl => self.registers.devaltc.modify(Devaltc::URTI1_SL::SET),
            ScfgField::I3c1Sl => self.registers.devalt10.modify(Devalt10::I3C1_SL::SET),
            ScfgField::I3c2Sl => self.registers.devalt10.modify(Devalt10::I3C2_SL::SET),
            ScfgField::I3c3Sl => self.registers.devalt10.modify(Devalt10::I3C3_SL::SET),
            ScfgField::I3c4Sl => self.registers.devalt10.modify(Devalt10::I3C4_SL::SET),
            ScfgField::I3c5Sl => self.registers.devalta.modify(Devalta::I3C5_SL::SET),
            ScfgField::I3c6Sl => {
                self.registers.devalt7.modify(Devalt7::I3C6_SL::SET);
                self.registers.devalt7.modify(Devalt7::GP96_SL::CLEAR);
                self.registers.devalt7.modify(Devalt7::GP97_SL::CLEAR);
                self.registers.devalt5.modify(Devalt5::PECI_EN::CLEAR);
            }
        }
    }

    pub fn clear_field(&self, field: ScfgField) {
        match field {
            ScfgField::Urto2Sl => self.registers.devalta.modify(Devalta::URTO2_SL::CLEAR),
            ScfgField::Urti1Sl => self.registers.devaltc.modify(Devaltc::URTI1_SL::CLEAR),
            ScfgField::I3c1Sl => self.registers.devalt10.modify(Devalt10::I3C1_SL::CLEAR),
            ScfgField::I3c2Sl => self.registers.devalt10.modify(Devalt10::I3C2_SL::CLEAR),
            ScfgField::I3c3Sl => self.registers.devalt10.modify(Devalt10::I3C3_SL::CLEAR),
            ScfgField::I3c4Sl => self.registers.devalt10.modify(Devalt10::I3C4_SL::CLEAR),
            ScfgField::I3c5Sl => self.registers.devalta.modify(Devalta::I3C5_SL::CLEAR),
            ScfgField::I3c6Sl => self.registers.devalt7.modify(Devalt7::I3C6_SL::CLEAR),
        }
    }

    /// Enable all I3C buses (I3C1-6)
    pub fn enable_all_i3c(&self) {
        self.registers.devalt10.modify(
            Devalt10::I3C1_SL::SET +
            Devalt10::I3C2_SL::SET +
            Devalt10::I3C3_SL::SET +
            Devalt10::I3C4_SL::SET
        );
        self.registers.devalta.modify(Devalta::I3C5_SL::SET);
        self.registers.devalt7.modify(Devalt7::I3C6_SL::SET);

        // I3C6 specific settings
        self.registers.devalt7.modify(Devalt7::I3C6_SL::SET);
        self.registers.devalt7.modify(Devalt7::GP96_SL::CLEAR);
        self.registers.devalt7.modify(Devalt7::GP97_SL::CLEAR);
        self.registers.devalt5.modify(Devalt5::PECI_EN::CLEAR);

        // Enable pull-up for all I3C SDA lines
        self.enable_i3c_pullup();
    }

    /// Enable I3C pull-up resistors for SDA lines of all buses
    pub fn enable_i3c_pullup(&self) {
        // Enable I3C1 pull-up (DEVPD1 bit 2)
        self.registers.devpd1.modify(Devpd1::I3C1_PUE::SET);
        // Enable I3C2-6 pull-ups (DEVPD3 bits 0-4)
        self.registers.devpd3.modify(
            Devpd3::I3C2_PUE::SET +
            Devpd3::I3C3_PUE::SET +
            Devpd3::I3C4_PUE::SET +
            Devpd3::I3C5_PUE::SET +
            Devpd3::I3C6_PUE::SET
        );
    }

    /// Disable I3C pull-up resistors for SDA lines of all buses
    pub fn disable_i3c_pullup(&self) {
        // Disable I3C1 pull-up (DEVPD1 bit 2)
        self.registers.devpd1.modify(Devpd1::I3C1_PUE::CLEAR);
        // Disable I3C2-6 pull-ups (DEVPD3 bits 0-4)
        self.registers.devpd3.modify(
            Devpd3::I3C2_PUE::CLEAR +
            Devpd3::I3C3_PUE::CLEAR +
            Devpd3::I3C4_PUE::CLEAR +
            Devpd3::I3C5_PUE::CLEAR +
            Devpd3::I3C6_PUE::CLEAR
        );
    }

    /// Enable pull-up for specific I3C bus (1-6)
    pub fn enable_i3c_pullup_bus(&self, bus: u8) {
        match bus {
            1 => self.registers.devpd1.modify(Devpd1::I3C1_PUE::SET),
            2 => self.registers.devpd3.modify(Devpd3::I3C2_PUE::SET),
            3 => self.registers.devpd3.modify(Devpd3::I3C3_PUE::SET),
            4 => self.registers.devpd3.modify(Devpd3::I3C4_PUE::SET),
            5 => self.registers.devpd3.modify(Devpd3::I3C5_PUE::SET),
            6 => self.registers.devpd3.modify(Devpd3::I3C6_PUE::SET),
            _ => {} // Invalid bus number, do nothing
        }
    }

    /// Disable pull-up for specific I3C bus (1-6)
    pub fn disable_i3c_pullup_bus(&self, bus: u8) {
        match bus {
            1 => self.registers.devpd1.modify(Devpd1::I3C1_PUE::CLEAR),
            2 => self.registers.devpd3.modify(Devpd3::I3C2_PUE::CLEAR),
            3 => self.registers.devpd3.modify(Devpd3::I3C3_PUE::CLEAR),
            4 => self.registers.devpd3.modify(Devpd3::I3C4_PUE::CLEAR),
            5 => self.registers.devpd3.modify(Devpd3::I3C5_PUE::CLEAR),
            6 => self.registers.devpd3.modify(Devpd3::I3C6_PUE::CLEAR),
            _ => {} // Invalid bus number, do nothing
        }
    }
}
