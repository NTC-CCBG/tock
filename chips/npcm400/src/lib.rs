// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

#![no_std]
#![crate_name = "npcm400"]
#![crate_type = "rlib"]

pub mod aes;
pub mod constants;
pub mod gpio;
pub mod peripheral_interrupts;
pub mod pinmux;
pub mod rtc;
pub mod temperature;
pub mod timer;
pub mod trng;

pub mod acomp;
pub mod adc;
pub mod approtect;
pub mod ble_radio;
pub mod chip;
pub mod clock;
pub mod crt1;
pub mod ficr;
pub mod i2c;
pub mod ieee802154_radio;
pub mod nvmc;
pub mod power;
pub mod ppi;
pub mod pwm;
pub mod spi;
pub mod uart;
pub mod uicr;
pub mod usbd;

pub use crate::crt1::init;
