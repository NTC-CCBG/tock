// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Component for starting up npcm400 platforms.
//! Contains 3 components, Npcm400fStartupComponent, Npcm400fClockComponent,
//! and UartChannelComponent, as well as two helper structs for
//! intializing Uart on Nordic boards.

use kernel::component::Component;
// use npcm400::gpio::Pin;
use npcm400::twd::WatchdogClient;

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
        self.clock.high_clock_on(npcm400::clock::HighClocks::ADC);
        self.clock.high_clock_on(npcm400::clock::HighClocks::ITIM1);
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

pub struct Npcm400fUartChannelComponent {
    uart1: &'static npcm400::uart::Uart<'static>,
}

impl Npcm400fUartChannelComponent {
    pub fn new(uart1: &'static npcm400::uart::Uart<'static>) -> Self {
        Self { uart1 }
    }
}

impl Component for Npcm400fUartChannelComponent {
    type StaticInput = ();
    type Output = &'static dyn kernel::hil::uart::Uart<'static>;

    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        self.uart1.initialize(115200);
        self.uart1
    }
}

pub struct Npcm400fWdtComponent {
    wdt: &'static npcm400::twd::Wdg<'static>,
}

impl Npcm400fWdtComponent {
    pub fn new(wdt: &'static npcm400::twd::Wdg<'static>) -> Self {
        Self { wdt }
    }
}

impl Component for Npcm400fWdtComponent {
    type StaticInput = ();
    type Output = &'static npcm400::twd::Wdg<'static>;

    fn finalize(self, _s: Self::StaticInput) -> Self::Output {
        // Create a static client instance

        // Set the client
        // self.wdt.set_client(client);

        // Initialize the watchdog
        self.wdt.finalize();

        self.wdt
    }
}
