// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2025.

//! PDMA (Peripheral Direct Memory Access) Controller for NPCM400
//!
//! The PDMA controller provides high-speed data transfer between memory and
//! peripherals without CPU intervention. Features:
//! - 14 independently configurable channels
//! - 2 priority levels (fixed priority or round-robin)
//! - Data sizes: 8, 16, 32 bits
//! - Supports Basic mode and Scatter-Gather mode
//! - Software and peripheral request support

use core::cell::Cell;
use kernel::utilities::cells::{OptionalCell, VolatileCell};
use kernel::utilities::registers::interfaces::{Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, ReadOnly, ReadWrite, WriteOnly};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

/// Base address for PDMA controller
const PDMA_BASE: StaticRef<PdmaRegisters> =
    unsafe { StaticRef::new(0x4001_5000 as *const PdmaRegisters) };

/// PDMA Register layout
#[repr(C)]
struct PdmaRegisters {
    // Descriptor Table Control Registers (0x000 - 0x0DC)
    dsct: [DescriptorTableRegs; 14],

    _reserved0: [u8; 0x20],

    // Current Scatter-Gather Descriptor Table Address Registers (0x100 - 0x134)
    curscat: [ReadOnly<u32>; 14],

    _reserved1: [u8; 0x2C8],

    // Control and Status Registers (0x400 onwards)
    chctl: ReadWrite<u32, CHCTL::Register>,     // 0x400
    stop: WriteOnly<u32>,                       // 0x404
    swreq: WriteOnly<u32>,                      // 0x408
    trgsts: ReadOnly<u32>,                      // 0x40C
    priset: ReadWrite<u32>,                     // 0x410
    priclr: WriteOnly<u32>,                     // 0x414
    inten: ReadWrite<u32>,                      // 0x418
    intsts: ReadWrite<u32, INTSTS::Register>,   // 0x41C
    abtsts: ReadWrite<u32, ABTSTS::Register>,   // 0x420
    tdsts: ReadWrite<u32, TDSTS::Register>,     // 0x424
    scatsts: ReadWrite<u32, SCATSTS::Register>, // 0x428
    tactsts: ReadOnly<u32>,                     // 0x42C
    _reserved2: [u8; 0xC],
    scatba: ReadWrite<u32>, // 0x43C
    _reserved3: [u8; 0x40],
    reqsel0_3: ReadWrite<u32, REQSEL::Register>,  // 0x480
    reqsel4_7: ReadWrite<u32, REQSEL::Register>,  // 0x484
    reqsel8_11: ReadWrite<u32, REQSEL::Register>, // 0x488
    reqsel12_13: ReadWrite<u32, REQSEL::Register>, // 0x48C
}

/// Descriptor Table Registers for each channel
#[repr(C)]
struct DescriptorTableRegs {
    ctl: ReadWrite<u32, DSCT_CTL::Register>, // Control
    endsa: ReadWrite<u32>,                   // End Source Address
    endda: ReadWrite<u32>,                   // End Destination Address
    next: ReadWrite<u32>,                    // Scatter-Gather Next Offset
}

register_bitfields![u32,
    /// Descriptor Table Control Register
    DSCT_CTL [
        /// Transfer Count (number of transfers)
        TXCNT OFFSET(16) NUMBITS(14) [],

        /// Request Source Selection
        /// 00 = Memory to Memory
        /// 01 = Peripheral to Memory
        /// 10 = Memory to Peripheral
        REQSRC OFFSET(13) NUMBITS(2) [
            MemToMem = 0,
            PeriphToMem = 1,
            MemToPeriph = 2
        ],

        /// Burst Size
        /// Number of transfers in a burst request
        /// 000 = 128 transfers
        /// 001 = 64 transfers
        /// 010 = 32 transfers
        /// 011 = 16 transfers
        /// 100 = 8 transfers
        /// 101 = 4 transfers
        /// 110 = 2 transfers
        /// 111 = 1 transfer
        BURSIZE OFFSET(10) NUMBITS(3) [
            Burst128 = 0,
            Burst64 = 1,
            Burst32 = 2,
            Burst16 = 3,
            Burst8 = 4,
            Burst4 = 5,
            Burst2 = 6,
            Burst1 = 7
        ],

        /// Destination Address Direction
        /// 00 = Increment
        /// 01 = Decrement
        /// 10 = Fixed
        DADIR OFFSET(8) NUMBITS(2) [
            Increment = 0,
            Decrement = 1,
            Fixed = 2
        ],

        /// Source Address Direction
        /// 00 = Increment
        /// 01 = Decrement
        /// 10 = Fixed
        SADIR OFFSET(6) NUMBITS(2) [
            Increment = 0,
            Decrement = 1,
            Fixed = 2
        ],

        /// Destination Address Increment Size
        /// 00 = One byte
        /// 01 = One half-word (2 bytes)
        /// 10 = One word (4 bytes)
        DAINC OFFSET(4) NUMBITS(2) [
            Byte = 0,
            HalfWord = 1,
            Word = 2
        ],

        /// Source Address Increment Size
        /// 00 = One byte
        /// 01 = One half-word (2 bytes)
        /// 10 = One word (4 bytes)
        SAINC OFFSET(2) NUMBITS(2) [
            Byte = 0,
            HalfWord = 1,
            Word = 2
        ],

        /// Operation Mode
        /// 00 = Stop (idle/finished)
        /// 01 = Basic mode
        /// 10 = Scatter-Gather mode
        OPMODE OFFSET(0) NUMBITS(2) [
            Stop = 0,
            Basic = 1,
            ScatterGather = 2
        ]
    ],

    /// Channel Control Register
    CHCTL [
        /// Channel Enable bits (one per channel 0-13)
        CH13EN OFFSET(13) NUMBITS(1) [],
        CH12EN OFFSET(12) NUMBITS(1) [],
        CH11EN OFFSET(11) NUMBITS(1) [],
        CH10EN OFFSET(10) NUMBITS(1) [],
        CH9EN OFFSET(9) NUMBITS(1) [],
        CH8EN OFFSET(8) NUMBITS(1) [],
        CH7EN OFFSET(7) NUMBITS(1) [],
        CH6EN OFFSET(6) NUMBITS(1) [],
        CH5EN OFFSET(5) NUMBITS(1) [],
        CH4EN OFFSET(4) NUMBITS(1) [],
        CH3EN OFFSET(3) NUMBITS(1) [],
        CH2EN OFFSET(2) NUMBITS(1) [],
        CH1EN OFFSET(1) NUMBITS(1) [],
        CH0EN OFFSET(0) NUMBITS(1) []
    ],

    /// Interrupt Status Register
    INTSTS [
        /// Transfer Done Interrupt Flag (channels 0-13)
        TDIF OFFSET(16) NUMBITS(14) [],
        /// Scatter-Gather Table Empty Interrupt Flag (channels 0-13)
        SGTDIF OFFSET(0) NUMBITS(14) []
    ],

    /// Abort Status Register
    ABTSTS [
        /// Target Abort Flag (channels 0-13)
        ABTIF OFFSET(0) NUMBITS(14) []
    ],

    /// Transfer Done Status Register
    TDSTS [
        /// Transfer Done Flag (channels 0-13)
        TDIF OFFSET(0) NUMBITS(14) []
    ],

    /// Scatter-Gather Transfer Done Status Register
    SCATSTS [
        /// Scatter-Gather Transfer Done Flag (channels 0-13)
        SGTDIF OFFSET(0) NUMBITS(14) []
    ],

    /// Request Source Selection Register
    REQSEL [
        /// Channel Selection (varies by register)
        REQSRC3 OFFSET(24) NUMBITS(5) [],
        REQSRC2 OFFSET(16) NUMBITS(5) [],
        REQSRC1 OFFSET(8) NUMBITS(5) [],
        REQSRC0 OFFSET(0) NUMBITS(5) []
    ]
];

/// PDMA Channel number (0-13)
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u8)]
pub enum PdmaChannel {
    Channel0 = 0,
    Channel1 = 1,
    Channel2 = 2,
    Channel3 = 3,
    Channel4 = 4,
    Channel5 = 5,
    Channel6 = 6,
    Channel7 = 7,
    Channel8 = 8,
    Channel9 = 9,
    Channel10 = 10,
    Channel11 = 11,
    Channel12 = 12,
    Channel13 = 13,
}

impl PdmaChannel {
    fn as_usize(&self) -> usize {
        *self as usize
    }

    fn bit_mask(&self) -> u32 {
        1 << (*self as u8)
    }
}

/// Peripheral request source selection for PDMA
/// Based on NPCM400 REQSEL register mappings
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u8)]
pub enum PdmaPeripheral {
    Reserved = 0x00,
    Spi1Tx = 0x01,
    Spi1Rx = 0x02,
    I3c1Rx = 0x05,
    I3c1Tx = 0x06,
    I3c2Rx = 0x07,
    I3c2Tx = 0x08,
    I3c3Rx = 0x09,
    I3c3Tx = 0x0A,
    I3c4Rx = 0x0B,
    I3c4Tx = 0x0C,
    I3c5Rx = 0x0D,
    I3c5Tx = 0x0E,
    I3c6Rx = 0x0F,
    I3c6Tx = 0x10,
}

/// Transfer data width
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaWidth {
    Byte = 0,     // 8-bit transfers
    HalfWord = 1, // 16-bit transfers
    Word = 2,     // 32-bit transfers
}

/// Transfer direction
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaDirection {
    MemToMem = 0,
    PeriphToMem = 1,
    MemToPeriph = 2,
}

/// Burst size selection for transfers
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaBurstSize {
    Burst128 = 0,
    Burst64 = 1,
    Burst32 = 2,
    Burst16 = 3,
    Burst8 = 4,
    Burst4 = 5,
    Burst2 = 6,
    Burst1 = 7,
}

/// Operation mode
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaMode {
    Stop = 0,
    Basic = 1,
    ScatterGather = 2,
}

/// Address increment mode
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaAddrMode {
    Increment = 0,
    Decrement = 1,
    Fixed = 2,
}

/// Memory-resident descriptor used by the PDMA scatter-gather engine
#[repr(C, align(4))]
pub struct PdmaScatterDescriptor {
    pub ctl: VolatileCell<u32>,
    pub endsa: VolatileCell<u32>,
    pub endda: VolatileCell<u32>,
    pub next: VolatileCell<u32>,
}

impl PdmaScatterDescriptor {
    /// Create a zeroed descriptor suitable for static allocation
    pub const fn new() -> Self {
        Self {
            ctl: VolatileCell::new(0),
            endsa: VolatileCell::new(0),
            endda: VolatileCell::new(0),
            next: VolatileCell::new(0),
        }
    }

    /// Populate the descriptor fields according to the transfer configuration
    pub fn configure(&self, config: &PdmaTransferConfig) -> Result<(), ErrorCode> {
        if config.mode == PdmaMode::Stop {
            return Err(ErrorCode::INVAL);
        }

        if config.mode == PdmaMode::ScatterGather && config.next.is_none() {
            return Err(ErrorCode::INVAL);
        }

        self.endsa.set(config.source_addr);
        self.endda.set(config.dest_addr);
        self.ctl.set(build_descriptor_control(config));
        self.next.set(config.next.unwrap_or(0));

        Ok(())
    }

    /// Reset the descriptor to an idle state
    pub fn clear(&self) {
        self.ctl.set(0);
        self.endsa.set(0);
        self.endda.set(0);
        self.next.set(0);
    }
}

/// PDMA transfer configuration
pub struct PdmaTransferConfig {
    pub source_addr: u32,
    pub dest_addr: u32,
    pub transfer_count: u16,
    pub src_width: PdmaWidth,
    pub dst_width: PdmaWidth,
    pub src_addr_mode: PdmaAddrMode,
    pub dst_addr_mode: PdmaAddrMode,
    pub direction: PdmaDirection,
    pub peripheral: Option<PdmaPeripheral>,
    pub burst_size: PdmaBurstSize,
    pub mode: PdmaMode,
    pub next: Option<u32>,
}

/// Client trait for PDMA callbacks
pub trait PdmaClient {
    /// Called when a transfer completes successfully
    fn transfer_done(&self, channel: PdmaChannel);

    /// Called when a transfer error occurs
    fn transfer_error(&self, channel: PdmaChannel, error: PdmaError);
}

/// PDMA error types
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PdmaError {
    TargetAbort,
    InvalidChannel,
    ChannelBusy,
    InvalidConfig,
}

/// PDMA Controller
pub struct Pdma {
    registers: StaticRef<PdmaRegisters>,
    clients: [OptionalCell<&'static dyn PdmaClient>; 14],
    channel_enabled: [Cell<bool>; 14],
}

impl Pdma {
    pub const fn new() -> Self {
        Self {
            registers: PDMA_BASE,
            clients: [
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
                OptionalCell::empty(),
            ],
            channel_enabled: [
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
                Cell::new(false),
            ],
        }
    }

    /// Set a client for a specific channel
    pub fn set_client(&self, channel: PdmaChannel, client: &'static dyn PdmaClient) {
        self.clients[channel.as_usize()].set(client);
    }

    /// Enable a PDMA channel
    pub fn enable_channel(&self, channel: PdmaChannel) {
        let regs = self.registers;
        let mask = channel.bit_mask();
        regs.chctl.set(regs.chctl.get() | mask);
        self.channel_enabled[channel.as_usize()].set(true);
    }

    /// Disable a PDMA channel
    pub fn disable_channel(&self, channel: PdmaChannel) {
        let regs = self.registers;
        regs.stop.set(channel.bit_mask());
        self.channel_enabled[channel.as_usize()].set(false);
    }

    /// Check if a channel is busy
    pub fn is_channel_busy(&self, channel: PdmaChannel) -> bool {
        let regs = self.registers;
        let tactsts = regs.tactsts.get();
        (tactsts & channel.bit_mask()) != 0
    }

    /// Configure and start a transfer (basic or scatter-gather)
    pub fn start_transfer(
        &self,
        channel: PdmaChannel,
        config: &PdmaTransferConfig,
    ) -> Result<(), ErrorCode> {
        if self.is_channel_busy(channel) {
            return Err(ErrorCode::BUSY);
        }

        if config.mode == PdmaMode::Stop {
            return Err(ErrorCode::INVAL);
        }

        if config.mode == PdmaMode::ScatterGather && config.next.is_none() {
            return Err(ErrorCode::INVAL);
        }

        let regs = self.registers;
        let ch_idx = channel.as_usize();
        let dsct = &regs.dsct[ch_idx];

        dsct.endsa.set(config.source_addr);
        dsct.endda.set(config.dest_addr);
        dsct.next.set(config.next.unwrap_or(0));
        dsct.ctl.set(build_descriptor_control(config));

        if let Some(periph) = config.peripheral {
            self.configure_peripheral_request(channel, periph);
        }

        let int_mask = channel.bit_mask();
        let mut inten_val = regs.inten.get() | (int_mask << 16);
        inten_val &= !int_mask; // Clear previous SG configuration for this channel

        if config.mode == PdmaMode::ScatterGather {
            inten_val |= int_mask; // Enable SGTDIF interrupt for this channel
        }

        regs.inten.set(inten_val);

        self.enable_channel(channel);

        Ok(())
    }

    /// Configure peripheral request source for a channel
    fn configure_peripheral_request(&self, channel: PdmaChannel, peripheral: PdmaPeripheral) {
        let regs = self.registers;
        let ch_num = channel.as_usize();
        let periph_val = peripheral as u32;

        match ch_num {
            0..=3 => {
                let shift = (ch_num % 4) * 8;
                let mask = !(0x1F << shift);
                let val = (regs.reqsel0_3.get() & mask) | (periph_val << shift);
                regs.reqsel0_3.set(val);
            }
            4..=7 => {
                let shift = (ch_num % 4) * 8;
                let mask = !(0x1F << shift);
                let val = (regs.reqsel4_7.get() & mask) | (periph_val << shift);
                regs.reqsel4_7.set(val);
            }
            8..=11 => {
                let shift = (ch_num % 4) * 8;
                let mask = !(0x1F << shift);
                let val = (regs.reqsel8_11.get() & mask) | (periph_val << shift);
                regs.reqsel8_11.set(val);
            }
            12..=13 => {
                let shift = (ch_num % 4) * 8;
                let mask = !(0x1F << shift);
                let val = (regs.reqsel12_13.get() & mask) | (periph_val << shift);
                regs.reqsel12_13.set(val);
            }
            _ => {}
        }
    }

    /// Set channel priority (fixed priority)
    pub fn set_fixed_priority(&self, channel: PdmaChannel, enable: bool) {
        let regs = self.registers;
        if enable {
            regs.priset.set(channel.bit_mask());
        } else {
            regs.priclr.set(channel.bit_mask());
        }
    }

    /// Trigger a software request for a channel
    pub fn trigger_software_request(&self, channel: PdmaChannel) {
        let regs = self.registers;
        regs.swreq.set(channel.bit_mask());
    }

    /// Handle interrupt for PDMA controller
    /// Should be called from the PDMA ISR
    pub fn handle_interrupt(&self) {
        let regs = self.registers;

        // Clear interrupt summary bits (both SG and TD)
        let intsts = regs.intsts.get();
        if intsts != 0 {
            regs.intsts.set(intsts);
        }

        let tdsts = regs.tdsts.get();
        if tdsts != 0 {
            regs.tdsts.set(tdsts);
        }

        let scatsts = regs.scatsts.get();
        if scatsts != 0 {
            regs.scatsts.set(scatsts);
        }

        let completion_mask = tdsts | scatsts;
        if completion_mask != 0 {
            for ch in 0..14 {
                if (completion_mask & (1 << ch)) != 0 {
                    if let Some(client) = self.clients[ch].get() {
                        let channel = match ch {
                            0 => PdmaChannel::Channel0,
                            1 => PdmaChannel::Channel1,
                            2 => PdmaChannel::Channel2,
                            3 => PdmaChannel::Channel3,
                            4 => PdmaChannel::Channel4,
                            5 => PdmaChannel::Channel5,
                            6 => PdmaChannel::Channel6,
                            7 => PdmaChannel::Channel7,
                            8 => PdmaChannel::Channel8,
                            9 => PdmaChannel::Channel9,
                            10 => PdmaChannel::Channel10,
                            11 => PdmaChannel::Channel11,
                            12 => PdmaChannel::Channel12,
                            13 => PdmaChannel::Channel13,
                            _ => continue,
                        };
                        client.transfer_done(channel);
                    }
                }
            }
        }

        let abtsts = regs.abtsts.get();
        if abtsts != 0 {
            // Clear the flags
            regs.abtsts.set(abtsts);

            // Notify clients of errors
            for ch in 0..14 {
                if (abtsts & (1 << ch)) != 0 {
                    if let Some(client) = self.clients[ch].get() {
                        let channel = match ch {
                            0 => PdmaChannel::Channel0,
                            1 => PdmaChannel::Channel1,
                            2 => PdmaChannel::Channel2,
                            3 => PdmaChannel::Channel3,
                            4 => PdmaChannel::Channel4,
                            5 => PdmaChannel::Channel5,
                            6 => PdmaChannel::Channel6,
                            7 => PdmaChannel::Channel7,
                            8 => PdmaChannel::Channel8,
                            9 => PdmaChannel::Channel9,
                            10 => PdmaChannel::Channel10,
                            11 => PdmaChannel::Channel11,
                            12 => PdmaChannel::Channel12,
                            13 => PdmaChannel::Channel13,
                            _ => continue,
                        };
                        client.transfer_error(channel, PdmaError::TargetAbort);
                    }
                }
            }
        }
    }

    /// Get scatter-gather base address
    pub fn get_scatter_gather_base(&self) -> u32 {
        self.registers.scatba.get()
    }

    /// Set scatter-gather base address
    pub fn set_scatter_gather_base(&self, addr: u32) {
        self.registers.scatba.set(addr);
    }
}

fn build_descriptor_control(config: &PdmaTransferConfig) -> u32 {
    let mut ctl_val = 0u32;

    // Transfer count (14 bits)
    ctl_val |= ((config.transfer_count as u32) & 0x3FFF) << 16;

    // Transfer direction selection
    ctl_val |= (config.direction as u32) << 13;

    // Burst size selection
    ctl_val |= (config.burst_size as u32) << 10;

    // Destination and source address direction controls
    ctl_val |= (config.dst_addr_mode as u32) << 8;
    ctl_val |= (config.src_addr_mode as u32) << 6;

    // Destination and source increment size controls
    ctl_val |= (config.dst_width as u32) << 4;
    ctl_val |= (config.src_width as u32) << 2;

    // Operation mode
    ctl_val |= config.mode as u32;

    ctl_val
}
