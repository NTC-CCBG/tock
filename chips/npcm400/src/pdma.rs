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
use kernel::utilities::registers::{
    register_bitfields, register_structs, ReadOnly, ReadWrite, WriteOnly,
};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

// Debug helper for PDMA driver
macro_rules! pdma_debug {
    ($($arg:tt)*) => (kernel::debug!($($arg)*));
}

/// Base address for PDMA controller
const PDMA_BASE: StaticRef<PdmaRegisters> =
    unsafe { StaticRef::new(0x4001_5000 as *const PdmaRegisters) };

register_structs! {
    DescriptorTableRegs {
        (0x000 => ctl: ReadWrite<u32, DSCT_CTL::Register>),
        (0x004 => endsa: ReadWrite<u32>),
        (0x008 => endda: ReadWrite<u32>),
        (0x00C => next: ReadWrite<u32>),
        (0x010 => @END),
    }
}

register_structs! {
    PdmaRegisters {
        (0x000 => dsct: [DescriptorTableRegs; 14]),
        (0x0E0 => _reserved0: [u8; 0x20]),
        (0x100 => curscat: [ReadOnly<u32>; 14]),
        (0x138 => _reserved1: [u8; 0x2C8]),
        (0x400 => chctl: ReadWrite<u32, CHCTL::Register>),
        (0x404 => stop: WriteOnly<u32>),
        (0x408 => swreq: WriteOnly<u32>),
        (0x40C => trgsts: ReadOnly<u32>),
        (0x410 => priset: ReadWrite<u32>),
        (0x414 => priclr: WriteOnly<u32>),
        (0x418 => inten: ReadWrite<u32>),
        (0x41C => intsts: ReadWrite<u32, INTSTS::Register>),
        (0x420 => abtsts: ReadWrite<u32, ABTSTS::Register>),
        (0x424 => tdsts: ReadWrite<u32, TDSTS::Register>),
        (0x428 => scatsts: ReadWrite<u32, SCATSTS::Register>),
        (0x42C => tactsts: ReadOnly<u32>),
        (0x430 => _reserved2: [u8; 0x0C]),
        (0x43C => scatba: ReadWrite<u32>),
        (0x440 => _reserved3: [u8; 0x40]),
        (0x480 => reqsel0_3: ReadWrite<u32, REQSEL::Register>),
        (0x484 => reqsel4_7: ReadWrite<u32, REQSEL::Register>),
        (0x488 => reqsel8_11: ReadWrite<u32, REQSEL::Register>),
        (0x48C => reqsel12_13: ReadWrite<u32, REQSEL::Register>),
        (0x490 => @END),
    }
}

register_bitfields![u32,
    /// Descriptor Table Control Register
    DSCT_CTL [
        /// Transfer Count (number of transfers - 1)
        TXCNT OFFSET(16) NUMBITS(14) [],

        /// Transfer Width (data size)
        /// 00 = 8-bit
        /// 01 = 16-bit
        /// 10 = 32-bit
        TXWIDTH OFFSET(12) NUMBITS(2) [
            Width8 = 0,
            Width16 = 1,
            Width32 = 2
        ],

        /// Destination Address Increment
        /// 00 = Increment
        /// 01 = Decrement
        /// 10 = Reserved
        /// 11 = Fixed (no increment)
        DAINC OFFSET(10) NUMBITS(2) [
            Increment = 0,
            Decrement = 1,
            Fixed = 3
        ],

        /// Source Address Increment
        /// 00 = Increment
        /// 01 = Decrement
        /// 10 = Reserved
        /// 11 = Fixed (no increment)
        SAINC OFFSET(8) NUMBITS(2) [
            Increment = 0,
            Decrement = 1,
            Fixed = 3
        ],

        /// Table Interrupt Disable
        TBINTDIS OFFSET(7) NUMBITS(1) [],

        /// Burst Size
        /// 000 = 128 transfers
        /// 001 = 64 transfers
        /// 010 = 32 transfers
        /// 011 = 16 transfers
        /// 100 = 8 transfers
        /// 101 = 4 transfers
        /// 110 = 2 transfers
        /// 111 = 1 transfer
        BURSIZE OFFSET(4) NUMBITS(3) [
            Burst128 = 0,
            Burst64 = 1,
            Burst32 = 2,
            Burst16 = 3,
            Burst8 = 4,
            Burst4 = 5,
            Burst2 = 6,
            Burst1 = 7
        ],

        /// Transfer Type
        /// 0 = Burst mode
        /// 1 = Single mode
        TXTYPE OFFSET(2) NUMBITS(1) [
            Burst = 0,
            Single = 1
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
        /// Table Empty Interrupt Status Flag
        TEIF OFFSET(2) NUMBITS(1) [],
        /// Transfer Done Interrupt Flag
        TDIF OFFSET(1) NUMBITS(1) [],
        /// Read/Write Target Abort Interrupt Status Flag
        ABTIF OFFSET(0) NUMBITS(1) []
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

        let ctl_value = build_descriptor_control(config);
        self.endsa.set(config.source_addr);
        self.endda.set(config.dest_addr);
        self.ctl.set(ctl_value);
        // Scatter-gather descriptors in RAM use full 32-bit NEXT address
        // (masking to 16-bit only applies to hardware channel descriptor)
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

    /// Return the remaining transfer count reported by the descriptor.
    ///
    /// When a scatter-gather transfer is active, the hardware updates the
    /// descriptor's TXCNT field with (remaining transfers - 1). Once the
    /// descriptor completes, the operation mode moves to `Stop` and we treat
    /// the remaining count as zero.
    pub fn remaining_transfers(&self) -> u16 {
        let ctl = self.ctl.get();

        // When the descriptor is no longer active the hardware clears OPMODE
        // back to `Stop`, which we interpret as no remaining transfers.
        if ctl == 0 || (ctl & 0x3) == PdmaMode::Stop as u32 {
            return 0;
        }

        let remaining_field = ((ctl >> 16) & 0x3FFF) as u16;
        remaining_field.saturating_add(1)
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
        pdma_debug!(
            "PDMA start_transfer - channel {:?}, count {}, mode {:?}, periph {:?}",
            channel,
            config.transfer_count,
            config.mode,
            config.peripheral
        );
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
    pub fn configure_peripheral_request(&self, channel: PdmaChannel, peripheral: PdmaPeripheral) {
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

    /// Initialize top descriptor table for scatter-gather mode
    ///
    /// This configures the hardware descriptor table (DSCT register) to point to
    /// a scatter-gather descriptor in RAM.
    ///
    /// # Arguments
    /// * `channel` - DMA channel to configure
    /// * `sg_desc_addr` - Physical address of the scatter-gather descriptor in RAM
    pub fn init_scatter_gather_descriptor(&self, channel: PdmaChannel, sg_desc_addr: u32) {
        let regs = self.registers;
        let ch_idx = channel.as_usize();
        let dsct = &regs.dsct[ch_idx];

        // Initialize top descriptor table with scatter-gather mode
        dsct.endsa.set(0x0);
        dsct.endda.set(0x0);
        dsct.next.set(sg_desc_addr & 0xFFFF); // Lower 16 bits

        // Set mode to scatter-gather (OPMODE = 2)
        dsct.ctl.set(2 << 0); // Only set OPMODE field, rest will be in SG descriptor
    }

    /// Clear transfer done status for a channel
    pub fn clear_transfer_done(&self, channel: PdmaChannel) {
        let ch_mask = channel.bit_mask();
        if self.registers.tdsts.get() & ch_mask != 0 {
            self.registers.tdsts.set(ch_mask); // Write 1 to clear
        }
    }

    /// Enable interrupts for a channel (both transfer done and scatter-gather done)
    pub fn enable_interrupts(&self, channel: PdmaChannel) {
        let regs = self.registers;
        let ch_mask = channel.bit_mask();
        let mut inten_val = regs.inten.get();
        // inten_val |= ch_mask << 16; // Enable TDIF (transfer done interrupt)
        inten_val |= ch_mask; // Enable SGTDIF (scatter-gather done interrupt)
        regs.inten.set(inten_val);
    }

    /// Read and display the current hardware descriptor table values for debugging
    pub fn debug_descriptor(&self, channel: PdmaChannel) {
        let regs = self.registers;
        let ch_idx = channel.as_usize();
        let dsct = &regs.dsct[ch_idx];

        let ctl = dsct.ctl.get();
        let sa = dsct.endsa.get();
        let da = dsct.endda.get();
        let next = dsct.next.get();

        pdma_debug!(
            "PDMA HW descriptor[{:?}]: CTL=0x{:08x}, SA=0x{:08x}, DA=0x{:08x}, NEXT=0x{:08x}",
            channel,
            ctl,
            sa,
            da,
            next
        );
        pdma_debug!(
            "  SCATBA=0x{:08x}, CHCTL=0x{:08x}, TDSTS=0x{:08x}, TACTSTS=0x{:08x}",
            regs.scatba.get(),
            regs.chctl.get(),
            regs.tdsts.get(),
            regs.tactsts.get()
        );
    }

    /// Return the remaining number of transfers for a channel.
    ///
    /// The hardware decrements the TXCNT field as a transfer progresses.
    /// Reading the descriptor control register provides the current value.
    pub fn remaining_transfers(&self, channel: PdmaChannel) -> u16 {
        let dsct = &self.registers.dsct[channel.as_usize()];
        (((dsct.ctl.get() >> 16) & 0x3FFF) + 1) as u16
    }

    /// Handle interrupt for PDMA controller
    /// Should be called from the PDMA ISR
    pub fn handle_interrupt(&self) {
        let regs = self.registers;

        // Clear interrupt summary bits (both SG and TD)
        let intsts = regs.intsts.get();
        let tdsts = regs.tdsts.get();
        let scatsts = regs.scatsts.get();
        let abtsts = regs.abtsts.get();

        // gpio::set_debug_gpio86(true);

        // pdma_debug!(
        //     "PDMA handle_interrupt - intsts=0x{:x}, tdsts=0x{:x}, scatsts=0x{:x}, abtsts=0x{:x}",
        //     intsts,
        //     tdsts,
        //     scatsts,
        //     abtsts
        // );

        if intsts != 0 {
            regs.intsts.set(intsts);
        }

        if tdsts != 0 {
            regs.tdsts.set(tdsts);
        }

        if scatsts != 0 {
            regs.scatsts.set(scatsts);
        }

        let completion_mask = tdsts | scatsts;
        if completion_mask != 0 {
            for ch in 0..14u8 {
                if (completion_mask & (1 << ch)) != 0 {
                    if let Some(client) = self.clients[ch as usize].get() {
                        client.transfer_done(unsafe { core::mem::transmute(ch) });
                    }
                }
            }
        }

        let abtsts = regs.abtsts.get();
        if abtsts != 0 {
            regs.abtsts.set(abtsts);
            for ch in 0..14u8 {
                if (abtsts & (1 << ch)) != 0 {
                    if let Some(client) = self.clients[ch as usize].get() {
                        client.transfer_error(
                            unsafe { core::mem::transmute(ch) },
                            PdmaError::TargetAbort,
                        );
                    }
                }
            }
        }

        // gpio::set_debug_gpio86(false);
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

/// Singleton PDMA controller instance used across the chip crate.
pub static PDMA: Pdma = Pdma::new();

unsafe impl Sync for Pdma {}

fn build_descriptor_control(config: &PdmaTransferConfig) -> u32 {
    use kernel::utilities::registers::LocalRegisterCopy;

    let mut ctl = LocalRegisterCopy::<u32, DSCT_CTL::Register>::new(0);

    // Transfer count (14 bits) - hardware uses (count-1)
    ctl.write(DSCT_CTL::TXCNT.val(config.transfer_count as u32));

    // Transfer width - use source width (byte/halfword/word)
    // PdmaWidth: Byte=0, HalfWord=1, Word=2 matches TXWIDTH encoding
    ctl.modify(DSCT_CTL::TXWIDTH.val(config.src_width as u32));

    // Destination address increment control
    // PdmaAddrMode: Increment=0, Decrement=1, Fixed=2
    // DAINC: Increment=0, Decrement=1, Fixed=3
    let dainc = match config.dst_addr_mode {
        PdmaAddrMode::Increment => 0,
        PdmaAddrMode::Decrement => 1,
        PdmaAddrMode::Fixed => 3,
    };
    ctl.modify(DSCT_CTL::DAINC.val(dainc));

    // Source address increment control
    let sainc = match config.src_addr_mode {
        PdmaAddrMode::Increment => 0,
        PdmaAddrMode::Decrement => 1,
        PdmaAddrMode::Fixed => 3,
    };
    ctl.modify(DSCT_CTL::SAINC.val(sainc));

    // Burst size (3 bits)
    ctl.modify(DSCT_CTL::BURSIZE.val(config.burst_size as u32));

    // Transfer type - set to Single for single-request peripherals
    ctl.modify(DSCT_CTL::TXTYPE::Single);

    // Operation mode (Basic or ScatterGather)
    ctl.modify(DSCT_CTL::OPMODE.val(config.mode as u32));

    ctl.get()
}
