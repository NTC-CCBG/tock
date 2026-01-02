// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! GPIO driver for NPCM400
//!
//! Based on NPCM400 GPIO IP specification (VER. 02H)
//! NCT6694B GPIO ports implementation

use core::cell::Cell;
use enum_primitive::cast::FromPrimitive;
use enum_primitive::enum_from_primitive;
use kernel::hil;
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::registers::interfaces::{Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, register_structs, ReadOnly, ReadWrite};
use kernel::utilities::StaticRef;

/// GPIO port registers structure
/// Each GPIO port has 8 pins (some ports may have fewer)
#[repr(C)]
struct GpioRegisters {
    /// Port GPIOx Data Out (offset 00h)
    dout: ReadWrite<u8, DOUT::Register>,
    /// Port GPIOx Data In (offset 01h)
    din: ReadOnly<u8, DIN::Register>,
    /// Port GPIOx Direction (offset 02h)
    dir: ReadWrite<u8, DIR::Register>,
    /// Port GPIOx Pull-Up or Pull-Down Enable (offset 03h)
    pull: ReadWrite<u8, PULL::Register>,
    /// Port GPIOx Pull-Up/Down Selection (offset 04h)
    pud: ReadWrite<u8, PUD::Register>,
    _reserved0: u8,
    /// Port GPIOx Output Type (offset 06h)
    otype: ReadWrite<u8, OTYPE::Register>,
    _reserved1: [u8; 7],
    /// Port GPIOx Version (offset 0Eh)
    ver: ReadOnly<u8>,
}

register_bitfields![u8,
    DOUT [
        /// Pin 7 output value
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 output value
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 output value
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 output value
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 output value
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 output value
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 output value
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 output value
        PIN0 OFFSET(0) NUMBITS(1) []
    ],
    DIN [
        /// Pin 7 input value
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 input value
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 input value
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 input value
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 input value
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 input value
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 input value
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 input value
        PIN0 OFFSET(0) NUMBITS(1) []
    ],
    DIR [
        /// Pin 7 direction (0=input, 1=output)
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 direction (0=input, 1=output)
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 direction (0=input, 1=output)
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 direction (0=input, 1=output)
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 direction (0=input, 1=output)
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 direction (0=input, 1=output)
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 direction (0=input, 1=output)
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 direction (0=input, 1=output)
        PIN0 OFFSET(0) NUMBITS(1) []
    ],
    PULL [
        /// Pin 7 pull enable
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 pull enable
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 pull enable
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 pull enable
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 pull enable
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 pull enable
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 pull enable
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 pull enable
        PIN0 OFFSET(0) NUMBITS(1) []
    ],
    PUD [
        /// Pin 7 pull direction (0=pull-up, 1=pull-down)
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 pull direction (0=pull-up, 1=pull-down)
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 pull direction (0=pull-up, 1=pull-down)
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 pull direction (0=pull-up, 1=pull-down)
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 pull direction (0=pull-up, 1=pull-down)
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 pull direction (0=pull-up, 1=pull-down)
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 pull direction (0=pull-up, 1=pull-down)
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 pull direction (0=pull-up, 1=pull-down)
        PIN0 OFFSET(0) NUMBITS(1) []
    ],
    OTYPE [
        /// Pin 7 output type (0=push-pull, 1=open-drain)
        PIN7 OFFSET(7) NUMBITS(1) [],
        /// Pin 6 output type (0=push-pull, 1=open-drain)
        PIN6 OFFSET(6) NUMBITS(1) [],
        /// Pin 5 output type (0=push-pull, 1=open-drain)
        PIN5 OFFSET(5) NUMBITS(1) [],
        /// Pin 4 output type (0=push-pull, 1=open-drain)
        PIN4 OFFSET(4) NUMBITS(1) [],
        /// Pin 3 output type (0=push-pull, 1=open-drain)
        PIN3 OFFSET(3) NUMBITS(1) [],
        /// Pin 2 output type (0=push-pull, 1=open-drain)
        PIN2 OFFSET(2) NUMBITS(1) [],
        /// Pin 1 output type (0=push-pull, 1=open-drain)
        PIN1 OFFSET(1) NUMBITS(1) [],
        /// Pin 0 output type (0=push-pull, 1=open-drain)
        PIN0 OFFSET(0) NUMBITS(1) []
    ]
];

// GPIO Port Base Addresses from NPCM400 spec
const GPIO0_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_1000 as *const GpioRegisters) };
const GPIO1_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_3000 as *const GpioRegisters) };
const GPIO2_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_5000 as *const GpioRegisters) };
const GPIO3_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_7000 as *const GpioRegisters) };
const GPIO4_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_9000 as *const GpioRegisters) };
const GPIO5_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_B000 as *const GpioRegisters) };
const GPIO6_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_D000 as *const GpioRegisters) };
const GPIO7_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4008_F000 as *const GpioRegisters) };
const GPIO8_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_1000 as *const GpioRegisters) };
const GPIO9_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_3000 as *const GpioRegisters) };
const GPIOA_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_5000 as *const GpioRegisters) };
const GPIOB_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_7000 as *const GpioRegisters) };
const GPIOC_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_9000 as *const GpioRegisters) };
const GPIOD_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_B000 as *const GpioRegisters) };
const GPIOE_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_D000 as *const GpioRegisters) };
const GPIOF_BASE: StaticRef<GpioRegisters> =
    unsafe { StaticRef::new(0x4009_F000 as *const GpioRegisters) };

/// GPIO Port identifier (0-F)
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PortId {
    Port0 = 0x0,
    Port1 = 0x1,
    Port2 = 0x2,
    Port3 = 0x3,
    Port4 = 0x4,
    Port5 = 0x5,
    Port6 = 0x6,
    Port7 = 0x7,
    Port8 = 0x8,
    Port9 = 0x9,
    PortA = 0xA,
    PortB = 0xB,
    PortC = 0xC,
    PortD = 0xD,
    PortE = 0xE,
    PortF = 0xF,
}

/// GPIO Pin identifier
/// Format: port (4 bits) | pin (4 bits)
#[rustfmt::skip]
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PinId {
    // Port 0
    P00 = 0x00, P01 = 0x01, P02 = 0x02, P03 = 0x03,
    P04 = 0x04, P05 = 0x05, P06 = 0x06, P07 = 0x07,
    // Port 1
    P10 = 0x10, P11 = 0x11, P12 = 0x12, P13 = 0x13,
    P14 = 0x14, P15 = 0x15, P16 = 0x16, P17 = 0x17,
    // Port 2
    P20 = 0x20, P21 = 0x21, P22 = 0x22, P23 = 0x23,
    P24 = 0x24, P25 = 0x25, P26 = 0x26, P27 = 0x27,
    // Port 3
    P30 = 0x30, P31 = 0x31, P32 = 0x32, P33 = 0x33,
    P34 = 0x34, P35 = 0x35, P36 = 0x36, P37 = 0x37,
    // Port 4
    P40 = 0x40, P41 = 0x41, P42 = 0x42, P43 = 0x43,
    P44 = 0x44, P45 = 0x45, P46 = 0x46, P47 = 0x47,
    // Port 5
    P50 = 0x50, P51 = 0x51, P52 = 0x52, P53 = 0x53,
    P54 = 0x54, P55 = 0x55, P56 = 0x56, P57 = 0x57,
    // Port 6
    P60 = 0x60, P61 = 0x61, P62 = 0x62, P63 = 0x63,
    P64 = 0x64, P65 = 0x65, P66 = 0x66, P67 = 0x67,
    // Port 7
    P70 = 0x70, P71 = 0x71, P72 = 0x72, P73 = 0x73,
    P74 = 0x74, P75 = 0x75, P76 = 0x76, P77 = 0x77,
    // Port 8
    P80 = 0x80, P81 = 0x81, P82 = 0x82, P83 = 0x83,
    P84 = 0x84, P85 = 0x85, P86 = 0x86, P87 = 0x87,
    // Port 9
    P90 = 0x90, P91 = 0x91, P92 = 0x92, P93 = 0x93,
    P94 = 0x94, P95 = 0x95, P96 = 0x96, P97 = 0x97,
    // Port A
    PA0 = 0xA0, PA1 = 0xA1, PA2 = 0xA2, PA3 = 0xA3,
    PA4 = 0xA4, PA5 = 0xA5, PA6 = 0xA6, PA7 = 0xA7,
    // Port B
    PB0 = 0xB0, PB1 = 0xB1, PB2 = 0xB2, PB3 = 0xB3,
    PB4 = 0xB4, PB5 = 0xB5, PB6 = 0xB6, PB7 = 0xB7,
    // Port C
    PC0 = 0xC0, PC1 = 0xC1, PC2 = 0xC2, PC3 = 0xC3,
    PC4 = 0xC4, PC5 = 0xC5, PC6 = 0xC6, PC7 = 0xC7,
    // Port D
    PD0 = 0xD0, PD1 = 0xD1, PD2 = 0xD2, PD3 = 0xD3,
    PD4 = 0xD4, PD5 = 0xD5, PD6 = 0xD6, PD7 = 0xD7,
    // Port E
    PE0 = 0xE0, PE1 = 0xE1, PE2 = 0xE2, PE3 = 0xE3,
    PE4 = 0xE4, PE5 = 0xE5, PE6 = 0xE6, PE7 = 0xE7,
    // Port F
    PF0 = 0xF0, PF1 = 0xF1, PF2 = 0xF2, PF3 = 0xF3,
    PF4 = 0xF4, PF5 = 0xF5, PF6 = 0xF6, PF7 = 0xF7,
}

impl PinId {
    /// Extract the pin number (0-7) from the PinId
    pub fn get_pin_number(&self) -> u8 {
        (*self as u8) & 0x0F
    }

    /// Extract the port number from the PinId
    pub fn get_port_number(&self) -> u8 {
        (*self as u8) >> 4
    }

    /// Get the bit mask for this pin within its port
    pub fn get_pin_mask(&self) -> u8 {
        1 << self.get_pin_number()
    }
}

/// GPIO Port structure
pub struct Port {
    registers: StaticRef<GpioRegisters>,
}

impl Port {
    const fn new(registers: StaticRef<GpioRegisters>) -> Self {
        Self { registers }
    }

    /// Read the direction register bit for a pin
    fn get_direction(&self, pin_mask: u8) -> bool {
        (self.registers.dir.get() & pin_mask) != 0
    }

    /// Set direction bit for a pin (1=output, 0=input)
    fn set_direction(&self, pin_mask: u8, is_output: bool) {
        if is_output {
            self.registers.dir.set(self.registers.dir.get() | pin_mask);
        } else {
            self.registers.dir.set(self.registers.dir.get() & !pin_mask);
        }
    }

    /// Read input data for a pin
    fn read_input(&self, pin_mask: u8) -> bool {
        (self.registers.din.get() & pin_mask) != 0
    }

    /// Write output data for a pin
    fn write_output(&self, pin_mask: u8, value: bool) {
        if value {
            self.registers.dout.set(self.registers.dout.get() | pin_mask);
        } else {
            self.registers.dout.set(self.registers.dout.get() & !pin_mask);
        }
    }

    /// Read output data register for a pin
    fn read_output(&self, pin_mask: u8) -> bool {
        (self.registers.dout.get() & pin_mask) != 0
    }

    /// Set pull-up/down configuration
    fn set_pull(&self, pin_mask: u8, state: hil::gpio::FloatingState) {
        match state {
            hil::gpio::FloatingState::PullUp => {
                // Enable pull, set to pull-up (PUD=0)
                self.registers.pull.set(self.registers.pull.get() | pin_mask);
                self.registers.pud.set(self.registers.pud.get() & !pin_mask);
            }
            hil::gpio::FloatingState::PullDown => {
                // Enable pull, set to pull-down (PUD=1)
                self.registers.pull.set(self.registers.pull.get() | pin_mask);
                self.registers.pud.set(self.registers.pud.get() | pin_mask);
            }
            hil::gpio::FloatingState::PullNone => {
                // Disable pull
                self.registers.pull.set(self.registers.pull.get() & !pin_mask);
            }
        }
    }

    /// Get pull-up/down configuration
    fn get_pull(&self, pin_mask: u8) -> hil::gpio::FloatingState {
        let pull_enabled = (self.registers.pull.get() & pin_mask) != 0;
        if !pull_enabled {
            hil::gpio::FloatingState::PullNone
        } else {
            let is_pulldown = (self.registers.pud.get() & pin_mask) != 0;
            if is_pulldown {
                hil::gpio::FloatingState::PullDown
            } else {
                hil::gpio::FloatingState::PullUp
            }
        }
    }

    /// Set output type (push-pull or open-drain)
    fn set_output_type(&self, pin_mask: u8, open_drain: bool) {
        if open_drain {
            self.registers.otype.set(self.registers.otype.get() | pin_mask);
        } else {
            self.registers.otype.set(self.registers.otype.get() & !pin_mask);
        }
    }
}

/// GPIO Ports collection
pub struct GpioPorts<'a> {
    ports: [Port; 16],
    pins: [[Option<Pin<'a>>; 8]; 16],
}

impl<'a> GpioPorts<'a> {
    pub fn new() -> Self {
        Self {
            ports: [
                Port::new(GPIO0_BASE),
                Port::new(GPIO1_BASE),
                Port::new(GPIO2_BASE),
                Port::new(GPIO3_BASE),
                Port::new(GPIO4_BASE),
                Port::new(GPIO5_BASE),
                Port::new(GPIO6_BASE),
                Port::new(GPIO7_BASE),
                Port::new(GPIO8_BASE),
                Port::new(GPIO9_BASE),
                Port::new(GPIOA_BASE),
                Port::new(GPIOB_BASE),
                Port::new(GPIOC_BASE),
                Port::new(GPIOD_BASE),
                Port::new(GPIOE_BASE),
                Port::new(GPIOF_BASE),
            ],
            pins: Self::initialize_pins(),
        }
    }

    /// Initialize all pins
    const fn initialize_pins() -> [[Option<Pin<'a>>; 8]; 16] {
        [
            // Port 0
            [
                Some(Pin::new(PinId::P00)),
                Some(Pin::new(PinId::P01)),
                Some(Pin::new(PinId::P02)),
                Some(Pin::new(PinId::P03)),
                Some(Pin::new(PinId::P04)),
                Some(Pin::new(PinId::P05)),
                Some(Pin::new(PinId::P06)),
                Some(Pin::new(PinId::P07)),
            ],
            // Port 1
            [
                Some(Pin::new(PinId::P10)),
                Some(Pin::new(PinId::P11)),
                Some(Pin::new(PinId::P12)),
                Some(Pin::new(PinId::P13)),
                Some(Pin::new(PinId::P14)),
                Some(Pin::new(PinId::P15)),
                Some(Pin::new(PinId::P16)),
                Some(Pin::new(PinId::P17)),
            ],
            // Port 2
            [
                Some(Pin::new(PinId::P20)),
                Some(Pin::new(PinId::P21)),
                Some(Pin::new(PinId::P22)),
                Some(Pin::new(PinId::P23)),
                Some(Pin::new(PinId::P24)),
                Some(Pin::new(PinId::P25)),
                Some(Pin::new(PinId::P26)),
                Some(Pin::new(PinId::P27)),
            ],
            // Port 3
            [
                Some(Pin::new(PinId::P30)),
                Some(Pin::new(PinId::P31)),
                Some(Pin::new(PinId::P32)),
                Some(Pin::new(PinId::P33)),
                Some(Pin::new(PinId::P34)),
                Some(Pin::new(PinId::P35)),
                Some(Pin::new(PinId::P36)),
                Some(Pin::new(PinId::P37)),
            ],
            // Port 4
            [
                Some(Pin::new(PinId::P40)),
                Some(Pin::new(PinId::P41)),
                Some(Pin::new(PinId::P42)),
                Some(Pin::new(PinId::P43)),
                Some(Pin::new(PinId::P44)),
                Some(Pin::new(PinId::P45)),
                Some(Pin::new(PinId::P46)),
                Some(Pin::new(PinId::P47)),
            ],
            // Port 5
            [
                Some(Pin::new(PinId::P50)),
                Some(Pin::new(PinId::P51)),
                Some(Pin::new(PinId::P52)),
                Some(Pin::new(PinId::P53)),
                Some(Pin::new(PinId::P54)),
                Some(Pin::new(PinId::P55)),
                Some(Pin::new(PinId::P56)),
                Some(Pin::new(PinId::P57)),
            ],
            // Port 6
            [
                Some(Pin::new(PinId::P60)),
                Some(Pin::new(PinId::P61)),
                Some(Pin::new(PinId::P62)),
                Some(Pin::new(PinId::P63)),
                Some(Pin::new(PinId::P64)),
                Some(Pin::new(PinId::P65)),
                Some(Pin::new(PinId::P66)),
                Some(Pin::new(PinId::P67)),
            ],
            // Port 7
            [
                Some(Pin::new(PinId::P70)),
                Some(Pin::new(PinId::P71)),
                Some(Pin::new(PinId::P72)),
                Some(Pin::new(PinId::P73)),
                Some(Pin::new(PinId::P74)),
                Some(Pin::new(PinId::P75)),
                Some(Pin::new(PinId::P76)),
                Some(Pin::new(PinId::P77)),
            ],
            // Port 8
            [
                Some(Pin::new(PinId::P80)),
                Some(Pin::new(PinId::P81)),
                Some(Pin::new(PinId::P82)),
                Some(Pin::new(PinId::P83)),
                Some(Pin::new(PinId::P84)),
                Some(Pin::new(PinId::P85)),
                Some(Pin::new(PinId::P86)),
                Some(Pin::new(PinId::P87)),
            ],
            // Port 9
            [
                Some(Pin::new(PinId::P90)),
                Some(Pin::new(PinId::P91)),
                Some(Pin::new(PinId::P92)),
                Some(Pin::new(PinId::P93)),
                Some(Pin::new(PinId::P94)),
                Some(Pin::new(PinId::P95)),
                Some(Pin::new(PinId::P96)),
                Some(Pin::new(PinId::P97)),
            ],
            // Port A
            [
                Some(Pin::new(PinId::PA0)),
                Some(Pin::new(PinId::PA1)),
                Some(Pin::new(PinId::PA2)),
                Some(Pin::new(PinId::PA3)),
                Some(Pin::new(PinId::PA4)),
                Some(Pin::new(PinId::PA5)),
                Some(Pin::new(PinId::PA6)),
                Some(Pin::new(PinId::PA7)),
            ],
            // Port B
            [
                Some(Pin::new(PinId::PB0)),
                Some(Pin::new(PinId::PB1)),
                Some(Pin::new(PinId::PB2)),
                Some(Pin::new(PinId::PB3)),
                Some(Pin::new(PinId::PB4)),
                Some(Pin::new(PinId::PB5)),
                Some(Pin::new(PinId::PB6)),
                Some(Pin::new(PinId::PB7)),
            ],
            // Port C
            [
                Some(Pin::new(PinId::PC0)),
                Some(Pin::new(PinId::PC1)),
                Some(Pin::new(PinId::PC2)),
                Some(Pin::new(PinId::PC3)),
                Some(Pin::new(PinId::PC4)),
                Some(Pin::new(PinId::PC5)),
                Some(Pin::new(PinId::PC6)),
                Some(Pin::new(PinId::PC7)),
            ],
            // Port D
            [
                Some(Pin::new(PinId::PD0)),
                Some(Pin::new(PinId::PD1)),
                Some(Pin::new(PinId::PD2)),
                Some(Pin::new(PinId::PD3)),
                Some(Pin::new(PinId::PD4)),
                Some(Pin::new(PinId::PD5)),
                Some(Pin::new(PinId::PD6)),
                Some(Pin::new(PinId::PD7)),
            ],
            // Port E
            [
                Some(Pin::new(PinId::PE0)),
                Some(Pin::new(PinId::PE1)),
                Some(Pin::new(PinId::PE2)),
                Some(Pin::new(PinId::PE3)),
                Some(Pin::new(PinId::PE4)),
                Some(Pin::new(PinId::PE5)),
                Some(Pin::new(PinId::PE6)),
                Some(Pin::new(PinId::PE7)),
            ],
            // Port F
            [
                Some(Pin::new(PinId::PF0)),
                Some(Pin::new(PinId::PF1)),
                Some(Pin::new(PinId::PF2)),
                Some(Pin::new(PinId::PF3)),
                Some(Pin::new(PinId::PF4)),
                Some(Pin::new(PinId::PF5)),
                Some(Pin::new(PinId::PF6)),
                Some(Pin::new(PinId::PF7)),
            ],
        ]
    }

    /// Get a pin by its PinId
    pub fn get_pin(&self, pinid: PinId) -> Option<&Pin<'a>> {
        let port_num = pinid.get_port_number() as usize;
        let pin_num = pinid.get_pin_number() as usize;
        self.pins[port_num][pin_num].as_ref()
    }

    /// Get a port by its PortId
    pub fn get_port(&self, portid: PortId) -> &Port {
        &self.ports[portid as usize]
    }

    /// Get a port by PinId
    pub fn get_port_from_pin(&self, pinid: PinId) -> &Port {
        &self.ports[pinid.get_port_number() as usize]
    }

    /// Setup circular dependencies for pins
    pub fn setup_circular_deps(&'a self) {
        for pin_group in self.pins.iter() {
            for pin in pin_group {
                pin.as_ref().map(|p| p.set_ports_ref(self));
            }
        }
    }
}

/// GPIO Pin structure
pub struct Pin<'a> {
    pinid: PinId,
    ports_ref: OptionalCell<&'a GpioPorts<'a>>,
    client: OptionalCell<&'a dyn hil::gpio::Client>,
}

impl<'a> Pin<'a> {
    pub const fn new(pinid: PinId) -> Self {
        Self {
            pinid,
            ports_ref: OptionalCell::empty(),
            client: OptionalCell::empty(),
        }
    }

    pub fn set_ports_ref(&self, ports: &'a GpioPorts<'a>) {
        self.ports_ref.set(ports);
    }

    pub fn get_pinid(&self) -> PinId {
        self.pinid
    }

    /// Get the port for this pin
    ///
    /// # Safety
    /// This function will panic if the ports_ref has not been initialized via setup_circular_deps().
    /// This indicates a board initialization bug where GPIO pins are being used without proper setup.
    fn get_port(&self) -> &Port {
        self.ports_ref
            .map(|ports| ports.get_port_from_pin(self.pinid))
            .unwrap_or_else(|| {
                panic!(
                    "GPIO pin {:?} accessed before setup_circular_deps() was called. \
                    This is a board initialization bug - GPIO pins must be properly initialized \
                    before use. Check that GpioPorts::setup_circular_deps() is called during \
                    platform initialization.",
                    self.pinid
                )
            })
    }

    /// Get the pin mask for this pin
    fn get_pin_mask(&self) -> u8 {
        self.pinid.get_pin_mask()
    }
}

impl<'a> hil::gpio::Configure for Pin<'a> {
    fn configuration(&self) -> hil::gpio::Configuration {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();

        if port.get_direction(pin_mask) {
            hil::gpio::Configuration::Output
        } else {
            hil::gpio::Configuration::Input
        }
    }

    fn make_output(&self) -> hil::gpio::Configuration {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();

        // Set direction to output
        port.set_direction(pin_mask, true);
        // Configure as push-pull by default
        port.set_output_type(pin_mask, false);

        hil::gpio::Configuration::Output
    }

    fn disable_output(&self) -> hil::gpio::Configuration {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();

        // Set direction to input
        port.set_direction(pin_mask, false);

        hil::gpio::Configuration::Input
    }

    fn make_input(&self) -> hil::gpio::Configuration {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();

        // Set direction to input
        port.set_direction(pin_mask, false);

        hil::gpio::Configuration::Input
    }

    fn disable_input(&self) -> hil::gpio::Configuration {
        // NPCM400 doesn't have a separate disable input state
        // Just return the current configuration
        self.configuration()
    }

    fn deactivate_to_low_power(&self) {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();

        // Set as input with no pull to minimize power
        port.set_direction(pin_mask, false);
        port.set_pull(pin_mask, hil::gpio::FloatingState::PullNone);
    }

    fn set_floating_state(&self, state: hil::gpio::FloatingState) {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        port.set_pull(pin_mask, state);
    }

    fn floating_state(&self) -> hil::gpio::FloatingState {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        port.get_pull(pin_mask)
    }
}

impl<'a> hil::gpio::Output for Pin<'a> {
    fn set(&self) {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        port.write_output(pin_mask, true);
    }

    fn clear(&self) {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        port.write_output(pin_mask, false);
    }

    fn toggle(&self) -> bool {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        let current = port.read_output(pin_mask);
        let new_value = !current;
        port.write_output(pin_mask, new_value);
        new_value
    }
}

impl<'a> hil::gpio::Input for Pin<'a> {
    fn read(&self) -> bool {
        let port = self.get_port();
        let pin_mask = self.get_pin_mask();
        port.read_input(pin_mask)
    }
}

impl<'a> hil::gpio::Interrupt<'a> for Pin<'a> {
    fn set_client(&self, client: &'a dyn hil::gpio::Client) {
        self.client.set(client);
    }

    fn enable_interrupts(&self, _mode: hil::gpio::InterruptEdge) {
        // TODO: Implement interrupt support via MIWU (Multi-Input Wake-Up Unit)
        // For now, interrupts are not supported
    }

    fn disable_interrupts(&self) {
        // TODO: Implement interrupt support via MIWU
    }

    fn is_pending(&self) -> bool {
        // TODO: Implement interrupt support via MIWU
        false
    }
}

/// Debug GPIO helper functions for GPIOs 86, 87, 94, 95
/// These functions provide low-level access to specific GPIOs for debugging purposes

/// Helper function to get the port base address for a given port number
const fn get_port_base(port_num: u8) -> StaticRef<GpioRegisters> {
    match port_num {
        0 => GPIO0_BASE,
        1 => GPIO1_BASE,
        2 => GPIO2_BASE,
        3 => GPIO3_BASE,
        4 => GPIO4_BASE,
        5 => GPIO5_BASE,
        6 => GPIO6_BASE,
        7 => GPIO7_BASE,
        8 => GPIO8_BASE,
        9 => GPIO9_BASE,
        0xA => GPIOA_BASE,
        0xB => GPIOB_BASE,
        0xC => GPIOC_BASE,
        0xD => GPIOD_BASE,
        0xE => GPIOE_BASE,
        0xF => GPIOF_BASE,
        _ => GPIO0_BASE, // Default fallback
    }
}

/// Helper function to configure and set a debug GPIO
fn configure_debug_gpio(pinid: PinId, initial_value: bool) {
    let port_base = get_port_base(pinid.get_port_number());
    let port = Port::new(port_base);
    let pin_mask = pinid.get_pin_mask();

    port.write_output(pin_mask, initial_value);
    port.set_direction(pin_mask, true); // Set as output
}

/// Helper function to set a debug GPIO value
fn set_debug_gpio(pinid: PinId, high: bool) {
    let port_base = get_port_base(pinid.get_port_number());
    let port = Port::new(port_base);
    port.write_output(pinid.get_pin_mask(), high);
}

/// Enable debug GPIOs 86, 87, 94, and 95 by configuring their mux, direction, and initial value
pub fn enable_debug_gpio86_87_95_94() {
    unsafe {
        // Configure pin multiplexing for debug GPIOs
        // GPIO86 - Port 8, Pin 6
        *(0x400C_3011_i32 as *mut u8) &= !0x80_u8; // mux: b7
        // GPIO87 - Port 8, Pin 7
        *(0x400C_3011_i32 as *mut u8) &= !0x20_u8; // mux: b5
        // GPIO95 - Port 9, Pin 5
        *(0x400C_3011_i32 as *mut u8) &= !0x18_u8; // mux: b4,b3
        // GPIO94 - Port 9, Pin 4
        *(0x400C_3011_i32 as *mut u8) &= !0x04_u8; // mux: b2
    }

    // Configure GPIOs as outputs with initial low value
    configure_debug_gpio(PinId::P86, false);
    configure_debug_gpio(PinId::P87, false);
    configure_debug_gpio(PinId::P95, false);
    configure_debug_gpio(PinId::P94, false);
}

/// Set GPIO86 to high or low
pub fn set_debug_gpio86(high: bool) {
    set_debug_gpio(PinId::P86, high);
}

/// Set GPIO87 to high or low
pub fn set_debug_gpio87(high: bool) {
    set_debug_gpio(PinId::P87, high);
}

/// Set GPIO94 to high or low
pub fn set_debug_gpio94(high: bool) {
    set_debug_gpio(PinId::P94, high);
}

/// Set GPIO95 to high or low
pub fn set_debug_gpio95(high: bool) {
    set_debug_gpio(PinId::P95, high);
}
