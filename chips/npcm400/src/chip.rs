// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

use core::fmt::Write;
use cortexm4f::{nvic, CortexM4F, CortexMVariant};
use kernel::platform::chip::InterruptService;

pub struct NPCM400<'a, I: InterruptService + 'a> {
    mpu: cortexm4f::mpu::MPU,
    userspace_kernel_boundary: cortexm4f::syscall::SysCall,
    interrupt_service: &'a I,
}

impl<'a, I: InterruptService + 'a> NPCM400<'a, I> {
    pub unsafe fn new(interrupt_service: &'a I) -> Self {
        Self {
            mpu: cortexm4f::mpu::MPU::new(),
            userspace_kernel_boundary: cortexm4f::syscall::SysCall::new(),
            interrupt_service,
        }
    }
}

/// This struct, when initialized, instantiates all peripheral drivers for the nrf52.
///
/// If a board wishes to use only a subset of these peripherals, this
/// should not be used or imported, and a modified version should be
/// constructed manually in main.rs.
pub struct Npcm400DefaultPeripherals<'a> {
    pub clock: crate::clock::Clock,
    pub scfg: crate::scfg::Scfg,
    pub adc: crate::adc::Adc<'a>,
    pub uarte0: crate::uart::Uarte<'a>,
}

impl Npcm400DefaultPeripherals<'_> {
    pub fn new() -> Self {
        Self {
            clock: crate::clock::Clock::new(),
            scfg: crate::scfg::Scfg::new(),
            adc: crate::adc::Adc::new(3300),
            uarte0: crate::uart::Uarte::new(crate::uart::UARTE0_BASE),
        }
    }
    // Necessary for setting up circular dependencies
    pub fn init(&'static self) {

    }
}
impl kernel::platform::chip::InterruptService for Npcm400DefaultPeripherals<'_> {
    unsafe fn service_interrupt(&self, interrupt: u32) -> bool {
        match interrupt {
            // crate::peripheral_interrupts::GPIOTE => self.gpio_port.handle_interrupt(),
            crate::peripheral_interrupts::ADC => self.adc.handle_interrupt(),
            crate::peripheral_interrupts::UART0 => self.uarte0.handle_interrupt(),
            _ => return self.service_interrupt(interrupt),
        }
        true
    }
}

impl<'a, I: InterruptService + 'a> kernel::platform::chip::Chip for NPCM400<'a, I> {
    type MPU = cortexm4f::mpu::MPU;
    type UserspaceKernelBoundary = cortexm4f::syscall::SysCall;

    fn mpu(&self) -> &Self::MPU {
        &self.mpu
    }

    fn userspace_kernel_boundary(&self) -> &Self::UserspaceKernelBoundary {
        &self.userspace_kernel_boundary
    }

    fn service_pending_interrupts(&self) {
        unsafe {
            loop {
                if let Some(interrupt) = nvic::next_pending() {
                    if !self.interrupt_service.service_interrupt(interrupt) {
                        panic!("unhandled interrupt {}", interrupt);
                    }
                    let n = nvic::Nvic::new(interrupt);
                    n.clear_pending();
                    n.enable();
                } else {
                    break;
                }
            }
        }
    }

    fn has_pending_interrupts(&self) -> bool {
        unsafe { nvic::has_pending() }
    }

    fn sleep(&self) {
        unsafe {
            cortexm4f::support::wfi();
        }
    }

    unsafe fn atomic<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        cortexm4f::support::atomic(f)
    }

    unsafe fn print_state(&self, write: &mut dyn Write) {
        CortexM4F::print_cortexm_state(write);
    }
}
