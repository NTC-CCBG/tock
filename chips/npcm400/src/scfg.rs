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
        (0x000 => _reserved: [u8; 26]),
        (0x01A => devalta: ReadWrite<u8, Devalta::Register>),
        (0x01B => _reserved1: [u8; 1]),
        (0x01C => devaltc: ReadWrite<u8, Devaltc::Register>),
        (0x01D => @END),
    }
}

register_bitfields! [u8,
    Devalta [
        URTO2_SL OFFSET(7) NUMBITS(1) []
    ],
    Devaltc [
        URTI1_SL OFFSET(6) NUMBITS(1) []
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
        }
    }

    pub fn clear_field(&self, field: ScfgField) {
        match field {
            ScfgField::Urto2Sl => self.registers.devalta.modify(Devalta::URTO2_SL::CLEAR),
            ScfgField::Urti1Sl => self.registers.devaltc.modify(Devaltc::URTI1_SL::CLEAR),
        }
    }
}
