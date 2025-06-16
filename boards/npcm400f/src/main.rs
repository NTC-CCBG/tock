// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Board file for NPCM400F Kit development board
//!
//! - <https://www.st.com/en/evaluation-tools/npcm400f.html>

#![no_std]
// Disable this attribute when documenting, as a workaround for
// https://github.com/rust-lang/rust/issues/62184.
#![cfg_attr(not(doc), no_main)]
// #![deny(missing_docs)]

use core::ptr::addr_of;
use core::ptr::addr_of_mut;

use capsules_core::virtualizers::virtual_alarm::VirtualMuxAlarm;
// use capsules_extra::lsm303xx;
use capsules_system::process_printer::ProcessPrinterText;
use components::gpio::GpioComponent;
use kernel::capabilities;
use kernel::component::Component;
use kernel::hil::gpio::Configure;
use kernel::hil::gpio::Output;
use kernel::hil::led::LedHigh;
use kernel::hil::time::Counter;
use kernel::platform::watchdog::WatchDog;
use kernel::platform::{KernelResources, SyscallDriverLookup};
use kernel::scheduler::round_robin::RoundRobinSched;
use kernel::{create_capability, debug, static_init};
use npcm400::chip::Npcm400DefaultPeripherals;
use npcm400::twd;

pub mod startup;
pub use self::startup::Npcm400fWdtComponent;
pub use self::startup::{
    Npcm400fClockComponent, Npcm400fScfgComponent, Npcm400fUartChannelComponent,
};

/// Support routines for debugging I/O.
pub mod io;

// Unit Tests for drivers.
#[allow(dead_code)]
mod virtual_uart_rx_test;

// Number of concurrent processes this platform supports.
const NUM_PROCS: usize = 4;

// Actual memory for holding the active process structures.
static mut PROCESSES: [Option<&'static dyn kernel::process::Process>; NUM_PROCS] =
    [None; NUM_PROCS];

// Static reference to chip for panic dumps.
static mut CHIP: Option<&'static npcm400::chip::Npcm400<Npcm400DefaultPeripherals>> = None;
// Static reference to process printer for panic dumps.
static mut PROCESS_PRINTER: Option<&'static ProcessPrinterText> = None;

// How should the kernel respond when a process faults.
const FAULT_RESPONSE: capsules_system::process_policies::PanicFaultPolicy =
    capsules_system::process_policies::PanicFaultPolicy {};

/// Dummy buffer that causes the linker to reserve enough space for the stack.
#[no_mangle]
#[link_section = ".stack_buffer"]
pub static mut STACK_MEMORY: [u8; 0x1700] = [0; 0x1700];

/// A structure representing this platform that holds references to all
/// capsules for this platform.
struct NPCM400F {
    console: &'static capsules_core::console::Console<'static>,
    ipc: kernel::ipc::IPC<{ NUM_PROCS as u8 }>,

    scheduler: &'static RoundRobinSched<'static>,
    systick: cortexm4::systick::SysTick,
    watchdog: &'static twd::Wdg<'static>,
}

/// Mapping of integer syscalls to objects that implement syscalls.
impl SyscallDriverLookup for NPCM400F {
    fn with_driver<F, R>(&self, driver_num: usize, f: F) -> R
    where
        F: FnOnce(Option<&dyn kernel::syscall::SyscallDriver>) -> R,
    {
        match driver_num {
            capsules_core::console::DRIVER_NUM => f(Some(self.console)),
            kernel::ipc::DRIVER_NUM => f(Some(&self.ipc)),
            _ => f(None),
        }
    }
}

impl
    KernelResources<
        npcm400::chip::Npcm400<'static, npcm400::chip::Npcm400DefaultPeripherals<'static>>,
    > for NPCM400F
{
    type SyscallDriverLookup = Self;
    type SyscallFilter = ();
    type ProcessFault = ();
    type Scheduler = RoundRobinSched<'static>;
    type SchedulerTimer = cortexm4::systick::SysTick;
    type WatchDog = twd::Wdg<'static>;
    type ContextSwitchCallback = ();

    fn syscall_driver_lookup(&self) -> &Self::SyscallDriverLookup {
        self
    }
    fn syscall_filter(&self) -> &Self::SyscallFilter {
        &()
    }
    fn process_fault(&self) -> &Self::ProcessFault {
        &()
    }
    fn scheduler(&self) -> &Self::Scheduler {
        self.scheduler
    }
    fn scheduler_timer(&self) -> &Self::SchedulerTimer {
        &self.systick
    }
    fn watchdog(&self) -> &Self::WatchDog {
        self.watchdog
    }
    fn context_switch_callback(&self) -> &Self::ContextSwitchCallback {
        &()
    }
}

/// Helper function called during bring-up that configures multiplexed I/O.
unsafe fn set_pin_primary_functions() {
    // use npcm400::gpio::{AlternateFunction, Mode, PinId, PortId};

    // syscfg.enable_clock();
}

/// Helper function for miscellaneous peripheral functions
unsafe fn setup_peripherals() {
    cortexm4::nvic::Nvic::new(npcm400::nvic::CR_UART1).enable();
    cortexm4::nvic::Nvic::new(npcm400::nvic::MSWC_T0OUT).enable();
}

/// Main function.
///
/// This is in a separate, inline(never) function so that its stack frame is
/// removed when this function returns. Otherwise, the stack space used for
/// these static_inits is wasted.
#[inline(never)]
unsafe fn start() -> (
    &'static kernel::Kernel,
    NPCM400F,
    &'static npcm400::chip::Npcm400<'static, Npcm400DefaultPeripherals<'static>>,
) {
    npcm400::init();

    //----------------------------------------------------------------------
    // Initialize chip peripheral drivers
    //----------------------------------------------------------------------
    let peripherals = static_init!(Npcm400DefaultPeripherals, Npcm400DefaultPeripherals::new());

    peripherals.setup_circular_deps();

    set_pin_primary_functions();

    setup_peripherals();

    let board_kernel = static_init!(kernel::Kernel, kernel::Kernel::new(&*addr_of!(PROCESSES)));

    let chip = static_init!(
        npcm400::chip::Npcm400<Npcm400DefaultPeripherals>,
        npcm400::chip::Npcm400::new(peripherals)
    );
    CHIP = Some(chip);

    //----------------------------------------------------------------------
    // Capabilties
    //----------------------------------------------------------------------

    // Create capabilities that the board needs to call certain protected kernel
    // functions.
    let memory_allocation_capability = create_capability!(capabilities::MemoryAllocationCapability);
    let process_management_capability =
        create_capability!(capabilities::ProcessManagementCapability);

    //----------------------------------------------------------------------
    // Clock
    //----------------------------------------------------------------------

    Npcm400fClockComponent::new(&peripherals.clock).finalize(());

    //----------------------------------------------------------------------
    // Pinmux
    //----------------------------------------------------------------------

    // Set pinmux in scfg
    Npcm400fScfgComponent::new(&peripherals.scfg).finalize(());

    //----------------------------------------------------------------------
    // UART
    //----------------------------------------------------------------------

    let uart_channel = Npcm400fUartChannelComponent::new(&peripherals.uart1).finalize(());

    let uart_mux = components::console::UartMuxComponent::new(uart_channel, 115200)
        .finalize(components::uart_mux_component_static!());

    // `finalize()` configures the underlying USART, so we need to
    // tell `send_byte()` not to configure the USART again.
    (*addr_of_mut!(io::WRITER)).set_initialized();

    // Setup the console.
    let console = components::console::ConsoleComponent::new(
        board_kernel,
        capsules_core::console::DRIVER_NUM,
        uart_mux,
    )
    .finalize(components::console_component_static!());
    // Create the debugger object that handles calls to `debug!()`.
    const DEBUG_BUFFER_KB: usize = 4;
    components::debug_writer::DebugWriterComponent::new(uart_mux)
        .finalize(components::debug_writer_component_static!(DEBUG_BUFFER_KB));

    //----------------------------------------------------------------------
    // TWD
    //----------------------------------------------------------------------
    // Create and finalize the component
    let wdt_component = Npcm400fWdtComponent::new(&peripherals.wdt).finalize(());

    // Enable the watchdog
    wdt_component.enable();

    //----------------------------------------------------------------------
    // Alarm
    //----------------------------------------------------------------------

    // let tim2 = &peripherals.tim2;
    // let mux_alarm = components::alarm::AlarmMuxComponent::new(tim2)
    //     .finalize(components::alarm_mux_component_static!(npcm400::tim2::Tim2));

    // let alarm = components::alarm::AlarmDriverComponent::new(
    //     board_kernel,
    //     capsules_core::alarm::DRIVER_NUM,
    //     mux_alarm,
    // )
    // .finalize(components::alarm_component_static!(npcm400::tim2::Tim2));

    //----------------------------------------------------------------------
    // GPIO
    //----------------------------------------------------------------------

    // let gpio_ports = &peripherals.gpio_ports;
    // // GPIO
    // let gpio = GpioComponent::new(
    //     board_kernel,
    //     capsules_core::gpio::DRIVER_NUM,
    //     components::gpio_component_helper!(
    //         npcm400::gpio::Pin<'static>,
    //         // Left outer connector
    //         0 => gpio_ports.get_pin(npcm400::gpio::PinId::PC01).unwrap(),
    //         1 => gpio_ports.get_pin(npcm400::gpio::PinId::PC03).unwrap(),
    //         // 2 => gpio_ports.get_pin(npcm400::gpio::PinId::PA01).unwrap(),
    //         // 3 => gpio_ports.get_pin(npcm400::gpio::PinId::PA03).unwrap(),
    //         // 4 => gpio_ports.get_pin(npcm400::gpio::PinId::PF04).unwrap(),
    //         // 5 => gpio_ports.get_pin(npcm400::gpio::PinId::PA05).unwrap(),
    //         // 6 => gpio_ports.get_pin(npcm400::gpio::PinId::PA07).unwrap(),
    //         // 7 => gpio_ports.get_pin(npcm400::gpio::PinId::PC05).unwrap(),
    //         // 8 => gpio_ports.get_pin(npcm400::gpio::PinId::PB01).unwrap(),
    //         9 => gpio_ports.get_pin(npcm400::gpio::PinId::PE07).unwrap(),
    //         // 10 => gpio_ports.get_pin(npcm400::gpio::PinId::PE09).unwrap(),
    //         11 => gpio_ports.get_pin(npcm400::gpio::PinId::PE11).unwrap(),
    //         // 12 => gpio_ports.get_pin(npcm400::gpio::PinId::PE13).unwrap(),
    //         // 13 => gpio_ports.get_pin(npcm400::gpio::PinId::PE15).unwrap(),
    //         14 => gpio_ports.get_pin(npcm400::gpio::PinId::PB11).unwrap(),
    //         // 15 => gpio_ports.get_pin(npcm400::gpio::PinId::PB13).unwrap(),
    //         // 16 => gpio_ports.get_pin(npcm400::gpio::PinId::PB15).unwrap(),
    //         17 => gpio_ports.get_pin(npcm400::gpio::PinId::PD09).unwrap(),
    //         18 => gpio_ports.get_pin(npcm400::gpio::PinId::PD11).unwrap(),
    //         19 => gpio_ports.get_pin(npcm400::gpio::PinId::PD13).unwrap(),
    //         20 => gpio_ports.get_pin(npcm400::gpio::PinId::PD15).unwrap(),
    //         21 => gpio_ports.get_pin(npcm400::gpio::PinId::PC06).unwrap(),
    //         // Left inner connector
    //         22 => gpio_ports.get_pin(npcm400::gpio::PinId::PC00).unwrap(),
    //         23 => gpio_ports.get_pin(npcm400::gpio::PinId::PC02).unwrap(),
    //         24 => gpio_ports.get_pin(npcm400::gpio::PinId::PF02).unwrap(),
    //         // 25 => gpio_ports.get_pin(npcm400::gpio::PinId::PA00).unwrap(),
    //         // 26 => gpio_ports.get_pin(npcm400::gpio::PinId::PA02).unwrap(),
    //         // 27 => gpio_ports.get_pin(npcm400::gpio::PinId::PA04).unwrap(),
    //         // 28 => gpio_ports.get_pin(npcm400::gpio::PinId::PA06).unwrap(),
    //         // 29 => gpio_ports.get_pin(npcm400::gpio::PinId::PC04).unwrap(),
    //         30 => gpio_ports.get_pin(npcm400::gpio::PinId::PB00).unwrap(),
    //         31 => gpio_ports.get_pin(npcm400::gpio::PinId::PB02).unwrap(),
    //         32 => gpio_ports.get_pin(npcm400::gpio::PinId::PE08).unwrap(),
    //         33 => gpio_ports.get_pin(npcm400::gpio::PinId::PE10).unwrap(),
    //         34 => gpio_ports.get_pin(npcm400::gpio::PinId::PE12).unwrap(),
    //         // 35 => gpio_ports.get_pin(npcm400::gpio::PinId::PE14).unwrap(),
    //         36 => gpio_ports.get_pin(npcm400::gpio::PinId::PB10).unwrap(),
    //         // 37 => gpio_ports.get_pin(npcm400::gpio::PinId::PB12).unwrap(),
    //         // 38 => gpio_ports.get_pin(npcm400::gpio::PinId::PB14).unwrap(),
    //         39 => gpio_ports.get_pin(npcm400::gpio::PinId::PD08).unwrap(),
    //         40 => gpio_ports.get_pin(npcm400::gpio::PinId::PD10).unwrap(),
    //         41 => gpio_ports.get_pin(npcm400::gpio::PinId::PD12).unwrap(),
    //         42 => gpio_ports.get_pin(npcm400::gpio::PinId::PD14).unwrap(),
    //         43 => gpio_ports.get_pin(npcm400::gpio::PinId::PC07).unwrap(),
    //         // Right inner connector
    //         44 => gpio_ports.get_pin(npcm400::gpio::PinId::PF09).unwrap(),
    //         45 => gpio_ports.get_pin(npcm400::gpio::PinId::PF00).unwrap(),
    //         46 => gpio_ports.get_pin(npcm400::gpio::PinId::PC14).unwrap(),
    //         47 => gpio_ports.get_pin(npcm400::gpio::PinId::PE06).unwrap(),
    //         48 => gpio_ports.get_pin(npcm400::gpio::PinId::PE04).unwrap(),
    //         49 => gpio_ports.get_pin(npcm400::gpio::PinId::PE02).unwrap(),
    //         50 => gpio_ports.get_pin(npcm400::gpio::PinId::PE00).unwrap(),
    //         51 => gpio_ports.get_pin(npcm400::gpio::PinId::PB08).unwrap(),
    //         // 52 => &gpio_ports.get_pin(npcm400::gpio::PinId::PB06).unwrap(),
    //         53 => gpio_ports.get_pin(npcm400::gpio::PinId::PB04).unwrap(),
    //         54 => gpio_ports.get_pin(npcm400::gpio::PinId::PD07).unwrap(),
    //         55 => gpio_ports.get_pin(npcm400::gpio::PinId::PD05).unwrap(),
    //         56 => gpio_ports.get_pin(npcm400::gpio::PinId::PD03).unwrap(),
    //         57 => gpio_ports.get_pin(npcm400::gpio::PinId::PD01).unwrap(),
    //         58 => gpio_ports.get_pin(npcm400::gpio::PinId::PC12).unwrap(),
    //         59 => gpio_ports.get_pin(npcm400::gpio::PinId::PC10).unwrap(),
    //         60 => gpio_ports.get_pin(npcm400::gpio::PinId::PA14).unwrap(),
    //         61 => gpio_ports.get_pin(npcm400::gpio::PinId::PF06).unwrap(),
    //         62 => gpio_ports.get_pin(npcm400::gpio::PinId::PA12).unwrap(),
    //         63 => gpio_ports.get_pin(npcm400::gpio::PinId::PA10).unwrap(),
    //         64 => gpio_ports.get_pin(npcm400::gpio::PinId::PA08).unwrap(),
    //         65 => gpio_ports.get_pin(npcm400::gpio::PinId::PC08).unwrap(),
    //         // Right outer connector
    //         66 => gpio_ports.get_pin(npcm400::gpio::PinId::PF10).unwrap(),
    //         67 => gpio_ports.get_pin(npcm400::gpio::PinId::PF01).unwrap(),
    //         68 => gpio_ports.get_pin(npcm400::gpio::PinId::PC15).unwrap(),
    //         69 => gpio_ports.get_pin(npcm400::gpio::PinId::PC13).unwrap(),
    //         70 => gpio_ports.get_pin(npcm400::gpio::PinId::PE05).unwrap(),
    //         71 => gpio_ports.get_pin(npcm400::gpio::PinId::PE03).unwrap(),
    //         72 => gpio_ports.get_pin(npcm400::gpio::PinId::PE01).unwrap(),
    //         73 => gpio_ports.get_pin(npcm400::gpio::PinId::PB09).unwrap(),
    //         // 74 => gpio_ports.get_pin(npcm400::gpio::PinId::PB07).unwrap(),
    //         75 => gpio_ports.get_pin(npcm400::gpio::PinId::PB05).unwrap(),
    //         76 => gpio_ports.get_pin(npcm400::gpio::PinId::PB03).unwrap(),
    //         77 => gpio_ports.get_pin(npcm400::gpio::PinId::PD06).unwrap(),
    //         78 => gpio_ports.get_pin(npcm400::gpio::PinId::PD04).unwrap(),
    //         79 => gpio_ports.get_pin(npcm400::gpio::PinId::PD02).unwrap(),
    //         80 => gpio_ports.get_pin(npcm400::gpio::PinId::PD00).unwrap(),
    //         81 => gpio_ports.get_pin(npcm400::gpio::PinId::PC11).unwrap(),
    //         82 => gpio_ports.get_pin(npcm400::gpio::PinId::PA15).unwrap(),
    //         83 => gpio_ports.get_pin(npcm400::gpio::PinId::PA13).unwrap(),
    //         84 => gpio_ports.get_pin(npcm400::gpio::PinId::PA11).unwrap(),
    //         85 => gpio_ports.get_pin(npcm400::gpio::PinId::PA09).unwrap(),
    //         86 => gpio_ports.get_pin(npcm400::gpio::PinId::PC09).unwrap()
    //     ),
    // )
    // .finalize(components::gpio_component_static!(
    //     npcm400::gpio::Pin<'static>
    // ));

    //----------------------------------------------------------------------
    // PLATFORM SETUP, SCHEDULER, AND START KERNEL LOOP
    //----------------------------------------------------------------------

    let process_printer = components::process_printer::ProcessPrinterTextComponent::new()
        .finalize(components::process_printer_text_component_static!());
    PROCESS_PRINTER = Some(process_printer);

    // PROCESS CONSOLE
    // let process_console = components::process_console::ProcessConsoleComponent::new(
    //     board_kernel,
    //     uart_mux,
    //     (),
    //     process_printer,
    //     Some(cortexm4::support::reset),
    // )
    // .finalize(components::process_console_component_static!(
    //     npcm400::tim2::Tim2
    // ));
    // let _ = process_console.start();

    let scheduler = components::sched::round_robin::RoundRobinComponent::new(&*addr_of!(PROCESSES))
        .finalize(components::round_robin_component_static!(NUM_PROCS));

    let npcm400f = NPCM400F {
        console,
        ipc: kernel::ipc::IPC::new(
            board_kernel,
            kernel::ipc::DRIVER_NUM,
            &memory_allocation_capability,
        ),

        scheduler,
        systick: cortexm4::systick::SysTick::new_with_calibration(96_000_000),
        watchdog: &peripherals.wdt,
    };

    // // Optional kernel tests
    // //
    // // See comment in `boards/imix/src/main.rs`
    // virtual_uart_rx_test::run_virtual_uart_receive(mux_uart);

    debug!("Initialization complete. Entering main loop");

    // These symbols are defined in the linker script.
    extern "C" {
        /// Beginning of the ROM region containing app images.
        static _sapps: u8;
        /// End of the ROM region containing app images.
        static _eapps: u8;
        /// Beginning of the RAM region for app memory.
        static mut _sappmem: u8;
        /// End of the RAM region for app memory.
        static _eappmem: u8;
    }

    kernel::process::load_processes(
        board_kernel,
        chip,
        core::slice::from_raw_parts(
            core::ptr::addr_of!(_sapps),
            core::ptr::addr_of!(_eapps) as usize - core::ptr::addr_of!(_sapps) as usize,
        ),
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(_sappmem),
            core::ptr::addr_of!(_eappmem) as usize - core::ptr::addr_of!(_sappmem) as usize,
        ),
        &mut *addr_of_mut!(PROCESSES),
        &FAULT_RESPONSE,
        &process_management_capability,
    )
    .unwrap_or_else(|err| {
        debug!("Error loading processes!");
        debug!("{:?}", err);
    });

    (board_kernel, npcm400f, chip)
}

/// Main function called after RAM initialized.
#[no_mangle]
pub unsafe fn main() {
    let main_loop_capability = create_capability!(capabilities::MainLoopCapability);

    let (board_kernel, platform, chip) = start();

    debug!(
        "[NPCM400F] Starting kernel loop with {} processes",
        PROCESSES.iter().filter(|p| p.is_some()).count()
    );

    board_kernel.kernel_loop(&platform, chip, Some(&platform.ipc), &main_loop_capability);
}
