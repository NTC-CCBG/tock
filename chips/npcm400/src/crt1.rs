// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

use cortexm4f::{
    initialize_ram_jump_to_main, nvic, scb, unhandled_interrupt, CortexM4F, CortexMVariant,
};

/*
 * Adapted from crt1.c which was relicensed by the original author from
 * GPLv3 to Apache 2.0.
 * The original version of the file, under GPL can be found at
 * https://github.com/SoftwareDefinedBuildings/stormport/blob/rebase0/tos/platforms/storm/stormcrt1.c
 *
 * Copyright 2016, Michael Andersen <m.andersen@eecs.berkeley.edu>
 */

extern "C" {
    // _estack is not really a function, but it makes the types work
    // You should never actually invoke it!!
    fn _estack();
}

#[cfg_attr(
    all(target_arch = "arm", target_os = "none"),
    link_section = ".vectors"
)]
// used Ensures that the symbol is kept until the final binary
#[cfg_attr(all(target_arch = "arm", target_os = "none"), used)]
/// ARM Cortex M Vector Table
pub static BASE_VECTORS: [unsafe extern "C" fn(); 16] = [
    // Stack Pointer
    _estack,
    // Reset Handler
    initialize_ram_jump_to_main,
    // NMI
    unhandled_interrupt,
    // Hard Fault
    CortexM4F::HARD_FAULT_HANDLER,
    // Memory Management Fault
    unhandled_interrupt,
    // Bus Fault
    unhandled_interrupt,
    // Usage Fault
    unhandled_interrupt,
    // Reserved
    unhandled_interrupt,
    // Reserved
    unhandled_interrupt,
    // Reserved
    unhandled_interrupt,
    // Reserved
    unhandled_interrupt,
    // SVCall
    CortexM4F::SVC_HANDLER,
    // Reserved for Debug
    unhandled_interrupt,
    // Reserved
    unhandled_interrupt,
    // PendSv
    unhandled_interrupt,
    // SysTick
    CortexM4F::SYSTICK_HANDLER,
];

#[cfg_attr(
    all(target_arch = "arm", target_os = "none"),
    link_section = ".vectors"
)]
// used Ensures that the symbol is kept until the final binary
#[cfg_attr(all(target_arch = "arm", target_os = "none"), used)]
pub static IRQS: [unsafe extern "C" fn(); 82] = [CortexM4F::GENERIC_ISR; 82];

#[no_mangle]
pub unsafe extern "C" fn init() {
    cortexm4f::nvic::disable_all();
    cortexm4f::nvic::clear_all_pending();

    // Explicitly tell the core where Tock's vector table is located. If Tock is the
    // only thing on the chip then this is effectively a no-op. If, however, there is
    // a bootloader present then we want to ensure that the vector table is set
    // correctly for Tock. The bootloader _may_ set this for us, but it may not
    // so that any errors early in the Tock boot process trap back to the bootloader.
    // To be safe we unconditionally set the vector table.
    scb::set_vector_table_offset(BASE_VECTORS.as_ptr().cast::<()>());
    cortexm4f::nvic::enable_all();
}
