// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

#![no_std]
#![crate_name = "npcm400"]
#![crate_type = "rlib"]

pub mod chip;
pub mod crt1;
pub mod clock;
pub mod scfg;
pub mod gpio;

pub mod peripheral_interrupts;
pub mod uart;
pub mod adc;
pub mod rtc;

pub use crate::crt1::init;
