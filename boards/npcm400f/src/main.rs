// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Tock kernel for the Nordic Semiconductor nRF52840 development kit (DK).

#![no_std]
// Disable this attribute when documenting, as a workaround for
// https://github.com/rust-lang/rust/issues/62184.
#![cfg_attr(not(doc), no_main)]
// #![deny(missing_docs)]

use core::ptr::addr_of;
use core::ptr::addr_of_mut;

// use capsules_core::virtualizers::virtual_alarm::VirtualMuxAlarm;
// use capsules_extra::lsm303xx;
// use capsules_system::process_printer::ProcessPrinterText;
// use components::gpio::GpioComponent;
use kernel::component::Component;
// use kernel::hil::gpio::Configure;
// use kernel::hil::gpio::Output;
// use kernel::hil::led::LedHigh;
// use kernel::hil::time::Counter;
use kernel::platform::{KernelResources, SyscallDriverLookup};
use kernel::scheduler::round_robin::RoundRobinSched;
use kernel::{capabilities, create_capability, debug, static_init};

use npcm400::chip::Npcm400DefaultPeripherals;
use npcm400::gpio::Pin;

pub mod startup;

pub use self::startup::{Npcm400fClockComponent, Npcm400fScfgComponent, Npcm400fStartupComponent};
pub use self::startup::{UartChannel, UartChannelComponent, UartPins};

const UART_RTS: Option<Pin> = Some(Pin::P0_05);
const UART_TXD: Pin = Pin::P0_06;
const UART_CTS: Option<Pin> = Some(Pin::P0_07);
const UART_RXD: Pin = Pin::P0_08;

/// Debug Writer
pub mod io;

/// This platform's chip type:
pub type Chip = npcm400::chip::NPCM400<'static, Npcm400DefaultPeripherals<'static>>;

/// Number of concurrent processes this platform supports.
pub const NUM_PROCS: usize = 4;

/// Process array of this platform.
pub static mut PROCESSES: [Option<&'static dyn kernel::process::Process>; NUM_PROCS] =
    [None; NUM_PROCS];

static mut CHIP: Option<&'static npcm400::chip::NPCM400<Npcm400DefaultPeripherals>> = None;
static mut PROCESS_PRINTER: Option<&'static capsules_system::process_printer::ProcessPrinterText> =
    None;

//=====================================================
// TODO: Removed
/// Macro to write a u8 value to a memory-mapped register address.
macro_rules! write_reg8 {
    ($addr:expr, $val:expr) => {
        unsafe {
            *($addr as *mut u8) = $val as u8;
        }
    };
}
macro_rules! read_reg8 {
    ($addr:expr) => {
        unsafe { *($addr as *mut u8) }
    };
}

/// Helper to write a string to UART1.
fn uart_write_str(s: &str) {
    for &b in s.as_bytes() {
        write_reg8!(0x400C_4000_u32, b);
    }
}

/// Helper to configure GPIOA4.
fn setup_gpioa4(val: u8) {
    write_reg8!(0x400C_3013_u32, 0x00); // mux: b5
    write_reg8!(0x4009_5000_u32, val); // val: b4
    write_reg8!(0x4009_5002_u32, 0x10); // dir: b4
}
//=====================================================

/// Supported drivers by the platform
pub struct Platform {
    console: &'static capsules_core::console::Console<'static>,
    pub ipc: kernel::ipc::IPC<{ NUM_PROCS as u8 }>,
    scheduler: &'static RoundRobinSched<'static>,
    systick: cortexm4::systick::SysTick,
}

impl SyscallDriverLookup for Platform {
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

impl KernelResources<Chip> for Platform {
    type SyscallDriverLookup = Self;
    type SyscallFilter = ();
    type ProcessFault = ();
    type Scheduler = RoundRobinSched<'static>;
    type SchedulerTimer = cortexm4::systick::SysTick;
    type WatchDog = ();
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
        &()
    }
    fn context_switch_callback(&self) -> &Self::ContextSwitchCallback {
        &()
    }
}

/// This is in a separate, inline(never) function so that its stack frame is
/// removed when this function returns. Otherwise, the stack space used for
/// these static_inits is wasted.
#[inline(never)]
pub unsafe fn start() -> (
    &'static kernel::Kernel,
    Platform,
    &'static Chip
) {
    //--------------------------------------------------------------------------
    // INITIAL SETUP
    //--------------------------------------------------------------------------

    // Apply errata fixes and enable interrupts.
    npcm400::init();

    {
        // GPIOA4
        setup_gpioa4(0x10);

        // clock
        write_reg8!(0x400b_5002, 0x72);
        write_reg8!(0x400b_5004, 0x0b);
        write_reg8!(0x400b_5006, 0x82);

        write_reg8!(0x400b_5000, read_reg8!(0x400b_5000) | 0x1);
        while read_reg8!(0x400b_5000) & 0x80 != 0 {}

        write_reg8!(0x400b_5008, 0x00);
        write_reg8!(0x400b_5010, 0x07);
        write_reg8!(0x400b_5012, 0x00);
        write_reg8!(0x400b_5014, 0x00);

        write_reg8!(0x4000_d008, 0x00); // UART_PD

        // pinmux
        write_reg8!(0x400C_301A, 0x93);
        write_reg8!(0x400C_301B, 0x00);
        write_reg8!(0x400C_301C, 0x6E);

        // uart
        write_reg8!(0x400C_400E, 0x08); // UPSR
        write_reg8!(0x400C_400C, 0x33); // BAUD
        write_reg8!(0x400C_4008, 0x00); // UFRS
        write_reg8!(0x400C_4016, 0x01); // UFCTRL
        write_reg8!(0x400C_4004, 0x40); // UICTRL

        // write UTBUF
        uart_write_str("===NPCM400==\n");

        // GPIOA4
        setup_gpioa4(0x00);
    }

    // Initialize chip peripheral drivers
    let npcm400_peripherals =
        static_init!(Npcm400DefaultPeripherals, Npcm400DefaultPeripherals::new());

    // Set up circular peripheral dependencies.
    npcm400_peripherals.init();

    // Setup space to store the core kernel data structure.
    let board_kernel = static_init!(kernel::Kernel, kernel::Kernel::new(&*addr_of!(PROCESSES)));

    // Create (and save for panic debugging) a chip object to setup low-level
    // resources (e.g. MPU, systick).
    let chip = static_init!(Chip, npcm400::chip::NPCM400::new(npcm400_peripherals));
    CHIP = Some(chip);

    // Choose the channel for serial output. This board can be configured to use
    // either the Segger RTT channel or via UART with traditional TX/RX GPIO
    // pins.
    // let uart_channel = UartChannel::Pins(UartPins::new(UART_RTS, UART_TXD, UART_CTS, UART_RXD));

    //--------------------------------------------------------------------------
    // CAPABILITIES
    //--------------------------------------------------------------------------

    // Create capabilities that the board needs to call certain protected kernel
    // functions.
    let memory_allocation_capability = create_capability!(capabilities::MemoryAllocationCapability);

    //--------------------------------------------------------------------------
    // NRF CLOCK SETUP
    //--------------------------------------------------------------------------

    Npcm400fClockComponent::new(&npcm400_peripherals.clock).finalize(());

    //--------------------------------------------------------------------------
    // NRF SCFG SETUP
    //--------------------------------------------------------------------------

    Npcm400fScfgComponent::new(&npcm400_peripherals.scfg).finalize(());
    Npcm400fScfgComponent::new(&npcm400_peripherals.scfg).finalize(());

    //--------------------------------------------------------------------------
    // Console
    //--------------------------------------------------------------------------

    let uart_channel = UartChannelComponent::new(&npcm400_peripherals.uart1).finalize(());

    // Virtualize the UART channel for the console and for kernel debug.
    let uart_mux = components::console::UartMuxComponent::new(uart_channel, 115200)
        .finalize(components::uart_mux_component_static!());

    // Setup the serial console for userspace.
    let console = components::console::ConsoleComponent::new(
        board_kernel,
        capsules_core::console::DRIVER_NUM,
        uart_mux,
    )
    .finalize(components::console_component_static!());

    // Create the debugger object that handles calls to `debug!()`.
    components::debug_writer::DebugWriterComponent::new(uart_mux)
        .finalize(components::debug_writer_component_static!());

    //--------------------------------------------------------------------------
    // PLATFORM SETUP, SCHEDULER, AND START KERNEL LOOP
    //--------------------------------------------------------------------------

    let scheduler = components::sched::round_robin::RoundRobinComponent::new(&*addr_of!(PROCESSES))
        .finalize(components::round_robin_component_static!(NUM_PROCS));

    let platform = Platform {
        console,
        ipc: kernel::ipc::IPC::new(
            board_kernel,
            kernel::ipc::DRIVER_NUM,
            &memory_allocation_capability,
        ),
        scheduler,
        systick: cortexm4::systick::SysTick::new_with_calibration(96_000_000),
    };

    // debug!("Initialization complete. Entering main loop\r");
    // debug!("{}", &*addr_of!(npcm400::ficr::FICR_INSTANCE));

    (board_kernel, platform, chip)
}

/// Main function called after RAM initialized.
#[no_mangle]
pub unsafe fn main() {
    let main_loop_capability = create_capability!(capabilities::MainLoopCapability);

    let (board_kernel, platform, chip) = start();
    board_kernel.kernel_loop(&platform, chip, Some(&platform.ipc), &main_loop_capability);
}
