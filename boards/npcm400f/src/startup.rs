// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Component for starting up npcm400 platforms.
//! Contains 3 components, Npcm400fStartupComponent, Npcm400fClockComponent,
//! and UartChannelComponent, as well as two helper structs for
//! intializing Uart on Nordic boards.

use kernel::component::Component;
use npcm400::gpio::Pin;

pub struct Npcm400fStartupComponent<'a> {
    _phantom: core::marker::PhantomData<&'a ()>, // Keeps the lifetime
}

impl<'a> Npcm400fStartupComponent<'a> {
    pub fn new() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl Component for Npcm400fStartupComponent<'_> {
    type StaticInput = ();
    type Output = ();
    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        // Do nothing
    }
}

pub struct Npcm400fClockComponent<'a> {
    clock: &'a npcm400::clock::Clock,
}

impl<'a> Npcm400fClockComponent<'a> {
    pub fn new(clock: &'a npcm400::clock::Clock) -> Self {
        Self { clock }
    }
}

impl Component for Npcm400fClockComponent<'_> {
    type StaticInput = ();
    type Output = ();
    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        // Start all of the clocks. Low power operation will require a better
        // approach than this.
        self.clock.config_clock();
        self.clock.high_clock_on(npcm400::clock::HighClocks::UART);
    }
}

pub struct Npcm400fScfgComponent<'a> {
    scfg: &'a npcm400::scfg::Scfg,
}

impl<'a> Npcm400fScfgComponent<'a> {
    pub fn new(scfg: &'a npcm400::scfg::Scfg) -> Self {
        Self { scfg }
    }
}

impl Component for Npcm400fScfgComponent<'_> {
    type StaticInput = ();
    type Output = ();
    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        // Set pinmux
        // self.scfg.init();
        self.scfg.set_field(npcm400::scfg::ScfgField::Urto2Sl);
        self.scfg.set_field(npcm400::scfg::ScfgField::Urti1Sl);
    }
}

#[macro_export]
macro_rules! uart_channel_component_static {
    ($A:ty $(,)?) => {{
        components::segger_rtt_component_static!($A)
    }};
}

/// Pins for the UART
#[allow(dead_code)]
pub struct UartPins {
    rts: Option<Pin>,
    txd: Pin,
    cts: Option<Pin>,
    rxd: Pin,
}

impl UartPins {
    pub fn new(rts: Option<Pin>, txd: Pin, cts: Option<Pin>, rxd: Pin) -> Self {
        Self { rts, txd, cts, rxd }
    }
}

/// Uart chanel representation depends on whether USB debugging is
/// enabled.
pub enum UartChannel<'a> {
    Pins(UartPins),
    _Phantom(core::marker::PhantomData<&'a ()>),
}

pub struct UartChannelComponent {
    uart_channel: UartChannel<'static>,
    uart1: &'static npcm400::uart::Uart1<'static>,
}

impl UartChannelComponent {
    pub fn new(
        uart_channel: UartChannel<'static>,
        uart1: &'static npcm400::uart::Uart1<'static>,
    ) -> Self {
        Self {
            uart_channel,
            uart1,
        }
    }
}

impl Component for UartChannelComponent {
    type StaticInput = ();
    type Output = &'static dyn kernel::hil::uart::Uart<'static>;

    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        match self.uart_channel {
            UartChannel::Pins(_uart_pins) => {
                self.uart1.initialize();
                self.uart1
            }
            _ => {
                panic!("UartChannelComponent: Unsupported UART channel variant");
            }
        }
    }
}
