// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Chip trait setup.

use core::fmt::Write;
use cortexm4f::{CortexM4F, CortexMVariant};
use kernel::platform::chip::Chip;
use kernel::platform::chip::InterruptService;

use crate::nvic;

pub struct Npcm400<'a, I: InterruptService + 'a> {
    mpu: cortexm4f::mpu::MPU,
    userspace_kernel_boundary: cortexm4f::syscall::SysCall,
    interrupt_service: &'a I,
}

pub struct Npcm400DefaultPeripherals<'a> {
    pub clock: crate::clock::Clock,
    pub scfg: crate::scfg::Scfg,
    pub uart1: crate::uart::Uart<'a>,
    pub wdt: crate::twd::Wdg<'a>,
    pub itim: crate::itim::Itim<'a>,
}

impl Npcm400DefaultPeripherals<'_> {
    pub fn new() -> Self {
        let source_clock = crate::clock::Clock::new(crate::clock::SourceFrequency::_96M);
        let uart_clock = source_clock
            .get_clock_source(crate::clock::HighClocks::UART)
            .unwrap();
        let wdt0 = crate::twd::Wdg::new();
        let itim = crate::itim::Itim::new(crate::clock::HighClocks::ITIM1);
        Self {
            clock: source_clock,
            scfg: crate::scfg::Scfg::new(),
            uart1: crate::uart::Uart::new_uart1(uart_clock),
            wdt: wdt0,
            itim: itim,
        }
    }

    // Setup any circular dependencies and register deferred calls
    pub fn setup_circular_deps(&'static self) {
        // Set clock reference for ITIM
        self.itim.set_clock(&self.clock);
        // self.gpio_ports.setup_circular_deps();

        // kernel::deferred_call::DeferredCallClient::register(&self.uart1);
    }
}

impl InterruptService for Npcm400DefaultPeripherals<'_> {
    unsafe fn service_interrupt(&self, interrupt: u32) -> bool {
        match interrupt {
            // nvic::ADC => self.adc.handle_interrupt(),
            nvic::CR_UART1 => self.uart1.handle_interrupt(),
            nvic::MSWC_T0OUT => self.wdt.handle_interrupt(),
            nvic::ITIM32_1 => self.itim.handle_interrupt(),
            nvic::ITIM32_2 => self.itim.handle_interrupt(),
            nvic::ITIM32_3 => self.itim.handle_interrupt(),
            nvic::ITIM32_4 => self.itim.handle_interrupt(),
            nvic::ITIM32_5 => self.itim.handle_interrupt(),
            nvic::ITIM32_6 => self.itim.handle_interrupt(),
            _ => return false,
        }
        true
    }
}

impl<'a, I: InterruptService + 'a> Npcm400<'a, I> {
    pub unsafe fn new(interrupt_service: &'a I) -> Self {
        Self {
            mpu: cortexm4f::mpu::MPU::new(),
            userspace_kernel_boundary: cortexm4f::syscall::SysCall::new(),
            interrupt_service,
        }
    }
}

impl<'a, I: InterruptService + 'a> Chip for Npcm400<'a, I> {
    type MPU = cortexm4f::mpu::MPU;
    type UserspaceKernelBoundary = cortexm4f::syscall::SysCall;

    fn service_pending_interrupts(&self) {
        unsafe {
            loop {
                if let Some(interrupt) = cortexm4f::nvic::next_pending() {
                    if !self.interrupt_service.service_interrupt(interrupt) {
                        panic!("unhandled interrupt {}", interrupt);
                    }
                    let n = cortexm4f::nvic::Nvic::new(interrupt);
                    n.clear_pending();
                    n.enable();
                } else {
                    break;
                }
            }
        }
    }

    fn has_pending_interrupts(&self) -> bool {
        unsafe { cortexm4f::nvic::has_pending() }
    }

    fn mpu(&self) -> &cortexm4f::mpu::MPU {
        &self.mpu
    }

    fn userspace_kernel_boundary(&self) -> &cortexm4f::syscall::SysCall {
        &self.userspace_kernel_boundary
    }

    fn sleep(&self) {
        unsafe {
            cortexm4f::scb::unset_sleepdeep();
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
