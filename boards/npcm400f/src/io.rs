// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

use core::fmt::Write;
use kernel::debug::IoWrite;
use kernel::hil::uart;
use kernel::hil::uart::Configure;

use npcm400::clock::{Clock, HighClocks, SourceFrequency};
use npcm400::uart::{Uart1, UART1_BASE};

enum Writer {
    WriterUart(/* initialized */ bool),
}

static mut WRITER: Writer = Writer::WriterUart(false);

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> ::core::fmt::Result {
        self.write(s.as_bytes());
        Ok(())
    }
}

impl IoWrite for Writer {
    fn write(&mut self, buf: &[u8]) -> usize {
        match self {
            Writer::WriterUart(ref mut initialized) => {
                // Here, we create a second instance of the Uart1 struct.
                // This is okay because we only call this during a panic, and
                // we will never actually process the interrupts

                // let clock = Clock::new(SourceFrequency::_96M);
                
                let uart1 = Uart1::new(UART1_BASE);
                if !*initialized {
                    *initialized = true;
                    // let _ = uart1.configure(uart::Parameters {
                    //     baud_rate: 115200,
                    //     stop_bits: uart::StopBits::One,
                    //     parity: uart::Parity::None,
                    //     hw_flow_control: false,
                    //     width: uart::Width::Eight,
                    // }, );
                }

                // TODO: skip for now
                // for &c in buf {
                //     unsafe { uart1.send_byte(c) }
                //     while !uart1.irq_tx_complete() {}
                // }
            }
        }
        buf.len()
    }
}

#[cfg(not(test))]
#[panic_handler]
/// Panic handler
pub unsafe fn panic_fmt(pi: &core::panic::PanicInfo) -> ! {
    use core::ptr::{addr_of, addr_of_mut};
    use kernel::debug;
    use kernel::hil::led;
    use npcm400::gpio::Pin;

    use crate::CHIP;
    use crate::PROCESSES;
    use crate::PROCESS_PRINTER;

    // The nRF52840DK LEDs (see back of board)
    let led_kernel_pin = &npcm400::gpio::GPIOPin::new(Pin::P0_13);
    let led = &mut led::LedLow::new(led_kernel_pin);
    let writer = &mut *addr_of_mut!(WRITER);
    debug::panic(
        &mut [led],
        writer,
        pi,
        &cortexm4::support::nop,
        &*addr_of!(PROCESSES),
        &*addr_of!(CHIP),
        &*addr_of!(PROCESS_PRINTER),
    )
}
