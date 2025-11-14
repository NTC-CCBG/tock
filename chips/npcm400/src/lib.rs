// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Peripheral implementations for the STM32F3xx MCU.
//!
//! STM32F303: <https://www.st.com/en/microcontrollers-microprocessors/stm32f303.html>

#![crate_name = "npcm400"]
#![crate_type = "rlib"]
#![no_std]

pub mod chip;
pub mod nvic;

// Peripherals
// pub mod adc;
// pub mod dma;
// pub mod exti;
pub mod fiu;
// pub mod flash;
pub mod gpio;
pub mod i3c;
// pub mod i2c;
pub mod pdma;
// pub mod rcc;
// pub mod spi;
pub mod spim;
// pub mod syscfg;
pub mod itim;
pub mod uart;
// pub mod miwu;
pub mod clock;
pub mod scfg;
pub mod twd;

use cortexm4f::{initialize_ram_jump_to_main, scb, unhandled_interrupt, CortexM4F, CortexMVariant};

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
pub static BASE_VECTORS: [unsafe extern "C" fn(); 16] = [
    _estack,
    initialize_ram_jump_to_main,
    unhandled_interrupt,           // NMI
    CortexM4F::HARD_FAULT_HANDLER, // Hard Fault
    unhandled_interrupt,           // MemManage
    unhandled_interrupt,           // BusFault
    unhandled_interrupt,           // UsageFault
    unhandled_interrupt,
    unhandled_interrupt,
    unhandled_interrupt,
    unhandled_interrupt,
    CortexM4F::SVC_HANDLER, // SVC
    unhandled_interrupt,    // DebugMon
    unhandled_interrupt,
    unhandled_interrupt,        // PendSV
    CortexM4F::SYSTICK_HANDLER, // SysTick
];

// NPCM400 has total of 82 interrupts (INT0..INT81)
// Mapping corrected per NPCM400 datasheet.
#[cfg_attr(all(target_arch = "arm", target_os = "none"), link_section = ".irqs")]
#[cfg_attr(all(target_arch = "arm", target_os = "none"), used)]
pub static IRQS: [unsafe extern "C" fn(); 82] = [
    CortexM4F::GENERIC_ISR, // INT0  SMB2 module interrupt
    CortexM4F::GENERIC_ISR, // INT1  Reserved / I3CI2 module interrupt
    CortexM4F::GENERIC_ISR, // INT2  CAN1 interrupt line_0
    CortexM4F::GENERIC_ISR, // INT3  Host I/F PM Channel1-4 Output Buffer Empty
    CortexM4F::GENERIC_ISR, // INT4  Host I/F PM Channel1-4 Input Buffer Full
    unhandled_interrupt,    // INT5  Reserved
    unhandled_interrupt,    // INT6  Reserved
    CortexM4F::GENERIC_ISR, // INT7  Shared Memory / Mailbox host write or clear interrupt
    CortexM4F::GENERIC_ISR, // INT8  CAN1 interrupt line_1
    CortexM4F::GENERIC_ISR, // INT9  PECI event
    CortexM4F::GENERIC_ISR, // INT10 Debug Port 80 interrupt
    CortexM4F::GENERIC_ISR, // INT11 eSPI interrupt
    CortexM4F::GENERIC_ISR, // INT12 MSWC wake-up (MSWCI) or TWD system tick (T0OUT)
    CortexM4F::GENERIC_ISR, // INT13 USB2.0 DC interrupt / USBH1.1 interrupt
    CortexM4F::GENERIC_ISR, // INT14 FIU interrupt
    CortexM4F::GENERIC_ISR, // INT15 PKA interrupt / Reserved
    CortexM4F::GENERIC_ISR, // INT16 AES interrupt / Reserved
    CortexM4F::GENERIC_ISR, // INT17 LCT_INT or CAN2 interrupt line_0
    CortexM4F::GENERIC_ISR, // INT18 SPIP1_INT or SPIM_INT interrupt
    CortexM4F::GENERIC_ISR, // INT19 RNG interrupt
    CortexM4F::GENERIC_ISR, // INT20 SMB6 module or I3CI1 module interrupt
    CortexM4F::GENERIC_ISR, // INT21 ADC interrupt (ADCI)
    CortexM4F::GENERIC_ISR, // INT22 GDMA interrupt or PDMA interrupt
    CortexM4F::GENERIC_ISR, // INT23 CR_UART1
    CortexM4F::GENERIC_ISR, // INT24 MFT16-1 (MFT16_INT1 or MFT16_INT2)
    CortexM4F::GENERIC_ISR, // INT25 MFT16-2 (MFT16_INT1 or MFT16_INT2)
    CortexM4F::GENERIC_ISR, // INT26 MFT16-3 (MFT16_INT1 or MFT16_INT2)
    CortexM4F::GENERIC_ISR, // INT27 Legacy (KBC, PRT, UARTA-UARTF, CIR)
    CortexM4F::GENERIC_ISR, // INT28 Reserved / FLM interrupt
    CortexM4F::GENERIC_ISR, // INT29 ITIM32-1 interrupt
    CortexM4F::GENERIC_ISR, // INT30 ITIM32-2 interrupt
    CortexM4F::GENERIC_ISR, // INT31 ITIM32-3 interrupt
    CortexM4F::GENERIC_ISR, // INT32 ITIM32-4 interrupt
    CortexM4F::GENERIC_ISR, // INT33 ITIM32-5 interrupt
    CortexM4F::GENERIC_ISR, // INT34 ITIM32-6 interrupt
    CortexM4F::GENERIC_ISR, // INT35 SMB1 (with FIFO) module interrupt
    CortexM4F::GENERIC_ISR, // INT36 EMAC interrupt
    CortexM4F::GENERIC_ISR, // INT37 SMB3 module interrupt
    CortexM4F::GENERIC_ISR, // INT38 SMB4 module interrupt
    CortexM4F::GENERIC_ISR, // INT39 SMB5 module interrupt
    CortexM4F::GENERIC_ISR, // INT40 MIWU0 WKINTA_0
    CortexM4F::GENERIC_ISR, // INT41 MIWU0 WKINTB_0
    CortexM4F::GENERIC_ISR, // INT42 MIWU0 WKINTC_0
    CortexM4F::GENERIC_ISR, // INT43 MIWU0 WKINTD_0
    CortexM4F::GENERIC_ISR, // INT44 MIWU0 WKINTE_0
    CortexM4F::GENERIC_ISR, // INT45 MIWU0 WKINTF_0
    CortexM4F::GENERIC_ISR, // INT46 MIWU0 WKINTG_0
    CortexM4F::GENERIC_ISR, // INT47 MIWU0 WKINTH_0
    CortexM4F::GENERIC_ISR, // INT48 MIWU1 WKINTA_1
    CortexM4F::GENERIC_ISR, // INT49 MIWU1 WKINTB_1
    CortexM4F::GENERIC_ISR, // INT50 MIWU1 WKINTC_1
    CortexM4F::GENERIC_ISR, // INT51 MIWU1 WKINTD_1
    CortexM4F::GENERIC_ISR, // INT52 CAN2 interrupt line_1
    CortexM4F::GENERIC_ISR, // INT53 MIWU1 WKINTF_1
    CortexM4F::GENERIC_ISR, // INT54 MIWU1 WKINTG_1
    CortexM4F::GENERIC_ISR, // INT55 MIWU1 WKINTH_1
    CortexM4F::GENERIC_ISR, // INT56 MIWU2 WKINTE_2
    CortexM4F::GENERIC_ISR, // INT57 MIWU2 WKINTF_2
    CortexM4F::GENERIC_ISR, // INT58 MIWU2 WKINTG_2
    CortexM4F::GENERIC_ISR, // INT59 MIWU2 WKINTH_2
    CortexM4F::GENERIC_ISR, // INT60 MIWU2 WKINTA_2
    CortexM4F::GENERIC_ISR, // INT61 MIWU2 WKINTB_2
    CortexM4F::GENERIC_ISR, // INT62 MIWU2 WKINTC_2
    CortexM4F::GENERIC_ISR, // INT63 MIWU2 WKINTD_2
    CortexM4F::GENERIC_ISR, // INT64 I3CI1 module interrupt
    CortexM4F::GENERIC_ISR, // INT65 I3CI2 module interrupt
    CortexM4F::GENERIC_ISR, // INT66 I3CI3 module interrupt
    CortexM4F::GENERIC_ISR, // INT67 I3CI4 module interrupt
    CortexM4F::GENERIC_ISR, // INT68 I3CI5 module interrupt
    CortexM4F::GENERIC_ISR, // INT69 I3CI6 module interrupt
    CortexM4F::GENERIC_ISR, // INT70 SMB7 module interrupt
    CortexM4F::GENERIC_ISR, // INT71 SMB8 module interrupt
    CortexM4F::GENERIC_ISR, // INT72 SMB9 module interrupt
    CortexM4F::GENERIC_ISR, // INT73 SMB10 module interrupt
    CortexM4F::GENERIC_ISR, // INT74 SMB11 module interrupt
    CortexM4F::GENERIC_ISR, // INT75 SMB12 module interrupt
    CortexM4F::GENERIC_ISR, // INT76 CR_UART2
    CortexM4F::GENERIC_ISR, // INT77 CR_UART3
    CortexM4F::GENERIC_ISR, // INT78 CR_UART4
    CortexM4F::GENERIC_ISR, // INT79 Reserved / USBH1.1 interrupt
    CortexM4F::GENERIC_ISR, // INT80 Reserved / SIOX1
    CortexM4F::GENERIC_ISR, // INT81 Reserved / SIOX2
];

pub unsafe fn init() {
    cortexm4f::nvic::disable_all();
    cortexm4f::nvic::clear_all_pending();
    scb::set_vector_table_offset(BASE_VECTORS.as_ptr().cast::<()>());
    cortexm4f::nvic::enable_all();

    // Set BASEPRI to 0 to ensure that SVC exceptions are not masked and always have
    // a higher priority than HardFault.
    use core::arch::asm;
    asm!(
        "cpsid i
        mov r0, #32
        msr basepri, r0
        dsb
        isb
        cpsie i"
    );
}
