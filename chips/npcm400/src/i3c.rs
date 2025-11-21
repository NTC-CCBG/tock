// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2025.

//! I3C Target Driver for Nuvoton NPCM400
//!
//! This driver implements I3C target (slave) functionality for the NPCM400 chip.
//! Based on the Zephyr I3C NPCM driver implementation.
//!
//! Hardware features:
//! - I3C target mode with dynamic address assignment
//! - FIFO-based RX/TX (16 bytes each)
//! - Interrupt-driven operation
//! - Support for private read/write transfers
//! - In-Band Interrupt (IBI) support

use core::cell::Cell;
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::cells::TakeCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, register_structs, ReadOnly, ReadWrite};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

/// I3C Hardware Interface Layer (HIL) traits
///
/// NOTE: These traits are sourced from the Caliptra MCU kernel's i3c-driver crate
/// (caliptra-mcu-sw/runtime/kernel/drivers/i3c/src/hil.rs).
///
/// This is an embedded copy to allow standalone builds of the npcm400 chip without
/// dependencies on the caliptra-mcu-sw project. If the Caliptra kernel HIL changes,
/// this module should be updated to match.
pub mod hil {
    use core::result::Result;
    use kernel::ErrorCode;

    /// Provides information about an I3C target device.
    pub struct I3CTargetInfo {
        /// Could be assigned by the hardware or absent.
        pub static_addr: Option<u8>,
        /// Could be assigned by the controller or absent if
        /// static address is used, or device has not received the address yet.
        pub dynamic_addr: Option<u8>,
        /// Maximum length of data that will be received in a Write command.
        pub max_read_len: usize,
        /// Maximum length of data that can be sent in response to a Read command.
        pub max_write_len: usize,
    }

    pub trait TxClient {
        /// Called when the packet has been transmitted.
        fn send_done(&self, tx_buffer: &'static mut [u8], result: Result<(), ErrorCode>);
    }

    pub trait RxClient {
        /// Called when a complete MCTP packet is received and ready to be processed.
        fn receive_write(&self, rx_buffer: &'static mut [u8], len: usize);

        /// Called when the I3C Controller has requested a private Write by addressing the target
        /// and the driver needs buffer to receive the data.
        /// The client should call set_rx_buffer() to set the buffer.
        fn write_expected(&self);
    }

    pub trait I3CTarget<'a> {
        /// Set the client that will be called when the packet is transmitted.
        fn set_tx_client(&self, client: &'a dyn TxClient);

        /// Set the client that will be called when the packet is received.
        fn set_rx_client(&self, client: &'a dyn RxClient);

        /// Set the buffer that will be used for receiving Write packets.
        fn set_rx_buffer(&self, rx_buf: &'static mut [u8]);

        /// Queue a packet in response to a private Read.
        fn transmit_read(
            &self,
            tx_buf: &'static mut [u8],
            len: usize,
        ) -> Result<(), (ErrorCode, &'static mut [u8])>;

        /// Enable the I3C target device
        fn enable(&self);

        /// Disable the I3C target device
        fn disable(&self);

        /// Returns information about this I3C Target Device.
        fn get_device_info(&self) -> I3CTargetInfo;
    }
}

/// I3C Bus default characteristics
const BUS_CHARACTERISTICS_TARGET: u8 = 0x26; // Standard target BCR
const DEVICE_CHARACTERISTICS: u8 = 0xCC; // Standard device characteristics

/// Mandatory Data Byte for MCTP Pending Read notification
/// This MDB is sent in the IBI to notify the controller that MCTP data is ready to be read
pub const MDB_PENDING_READ_MCTP: u8 = 0xae;

// Conditional debug macro for I3C driver
#[cfg(feature = "debug-i3c")]
macro_rules! i3c_debug {
    ($($arg:tt)*) => (kernel::debug!($($arg)*));
}

#[cfg(not(feature = "debug-i3c"))]
macro_rules! i3c_debug {
    ($($arg:tt)*) => {{}};
}

/// I3C Provisioned ID (PID) - 48-bit unique device identifier
#[derive(Copy, Clone, Debug)]
pub struct ProvisionedId {
    /// Manufacturer ID (MIPI vendor ID) - 15 bits
    pub vendor_id: u16,
    /// Part ID - 16 bits
    pub part_id: u16,
    /// Instance ID - 4 bits
    pub instance_id: u8,
    /// Extra information (e.g., silicon revision) - 12 bits
    pub extra_info: u16,
}

impl ProvisionedId {
    /// Create a new Provisioned ID
    pub const fn new(vendor_id: u16, part_id: u16, instance_id: u8, extra_info: u16) -> Self {
        Self {
            vendor_id: vendor_id & 0x7FFF,   // 15 bits
            part_id,                         // 16 bits
            instance_id: instance_id & 0x0F, // 4 bits
            extra_info: extra_info & 0x0FFF, // 12 bits
        }
    }

    /// Get the lower 32 bits (Part ID + Instance + Extra)
    pub fn get_pid_low(&self) -> u32 {
        ((self.part_id as u32) << 16) | ((self.instance_id as u32) << 12) | (self.extra_info as u32)
    }

    /// Get the upper 16 bits (Vendor ID)
    pub fn get_pid_high(&self) -> u32 {
        (self.vendor_id as u32) & 0x7FFF
    }
}

/// I3C Bus Characteristics Register (BCR) - 8 bits
#[derive(Copy, Clone, Debug)]
pub struct BusCharacteristics {
    raw_value: u8,
}

impl BusCharacteristics {
    /// Create standard I3C target BCR
    pub const fn new_target() -> Self {
        Self {
            raw_value: BUS_CHARACTERISTICS_TARGET,
        }
    }

    /// Create BCR from raw 8-bit value
    pub const fn from_raw(value: u8) -> Self {
        Self { raw_value: value }
    }

    /// Convert to 8-bit register value
    pub fn to_u8(&self) -> u8 {
        self.raw_value
    }

    // Accessor methods for individual bits
    pub fn device_role(&self) -> bool {
        (self.raw_value & (1 << 7)) != 0
    }

    pub fn advanced_capabilities(&self) -> bool {
        (self.raw_value & (1 << 6)) != 0
    }

    pub fn virtual_target_support(&self) -> bool {
        (self.raw_value & (1 << 3)) != 0
    }

    pub fn offline_capable(&self) -> bool {
        (self.raw_value & (1 << 2)) != 0
    }

    pub fn ibi_payload(&self) -> bool {
        (self.raw_value & (1 << 1)) != 0
    }

    pub fn ibi_request_capable(&self) -> bool {
        (self.raw_value & 1) != 0
    }
}

/// I3C Device Characteristics Register (DCR) - 8 bits
/// Defines the device type/class
#[derive(Copy, Clone, Debug)]
pub struct DeviceCharacteristics {
    raw_value: u8,
}

impl DeviceCharacteristics {
    /// Generic I3C device
    pub const GENERIC: Self = Self { raw_value: 0x00 };
    /// Sensor - Accelerometer
    pub const SENSOR_ACCELEROMETER: Self = Self { raw_value: 0x01 };
    /// Sensor - Ambient Light
    pub const SENSOR_LIGHT_AMBIENT: Self = Self { raw_value: 0x02 };
    /// Vendor Specific
    pub const VENDOR_SPECIFIC: Self = Self { raw_value: 0xFF };
    /// Default
    pub const DEFAULT: Self = Self {
        raw_value: DEVICE_CHARACTERISTICS,
    };

    /// Create DCR from raw 8-bit value
    pub const fn from_raw(value: u8) -> Self {
        Self { raw_value: value }
    }

    /// Convert to 8-bit register value
    pub fn to_u8(&self) -> u8 {
        self.raw_value
    }
}

/// I3C register base addresses for NPCM400 (6 I3C controllers)
/// Base addresses from NPCM400 memory map
/// Updated based on actual hardware testing (devmem shows I3C1 at 0x40004a00)
const I3C1_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4000 as *const I3cRegisters) };
const I3C2_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4200 as *const I3cRegisters) };
const I3C3_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4400 as *const I3cRegisters) };
const I3C4_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4600 as *const I3cRegisters) };
const I3C5_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4800 as *const I3cRegisters) };
const I3C6_BASE: StaticRef<I3cRegisters> =
    unsafe { StaticRef::new(0x4000_4A00 as *const I3cRegisters) };

/// PDMA (Peripheral DMA) base address
const PDMA_BASE: StaticRef<PdmaRegisters> =
    unsafe { StaticRef::new(0x4001_5000 as *const PdmaRegisters) };

/// DMA channel assignments for I3C buses
/// Each I3C bus uses 2 DMA channels (RX and TX)
const I3C1_DMA_RX_CHANNEL: u8 = 0;
const I3C1_DMA_TX_CHANNEL: u8 = 1;
const I3C2_DMA_RX_CHANNEL: u8 = 2;
const I3C2_DMA_TX_CHANNEL: u8 = 3;
const I3C3_DMA_RX_CHANNEL: u8 = 4;
const I3C3_DMA_TX_CHANNEL: u8 = 5;
const I3C4_DMA_RX_CHANNEL: u8 = 6;
const I3C4_DMA_TX_CHANNEL: u8 = 7;
const I3C5_DMA_RX_CHANNEL: u8 = 8;
const I3C5_DMA_TX_CHANNEL: u8 = 9;
const I3C6_DMA_RX_CHANNEL: u8 = 10;
const I3C6_DMA_TX_CHANNEL: u8 = 11;

register_structs! {
    /// NPCM I3C Target Register Map
    /// Based on Zephyr i3c_npcm.h struct i3c_reg
    I3cRegisters {
        /// 0x000: Controller Configuration (not used in target mode)
        (0x000 => mconfig: ReadWrite<u32, MCONFIG::Register>),
        /// 0x004: Target Configuration
        (0x004 => config: ReadWrite<u32, CONFIG::Register>),
        /// 0x008: Target Status
        (0x008 => status: ReadWrite<u32, STATUS::Register>),
        /// 0x00C: Target I3C Control
        (0x00C => ctrl: ReadWrite<u32, CTRL::Register>),
        /// 0x010: Target Interrupt Enable Set
        (0x010 => intset: ReadWrite<u32, INTSET::Register>),
        /// 0x014: Target Interrupt Enable Clear
        (0x014 => intclr: ReadWrite<u32, INTCLR::Register>),
        /// 0x018: Target Interrupt Masked (status)
        (0x018 => intmasked: ReadOnly<u32, INTMASKED::Register>),
        /// 0x01C: Target Error and Warning
        (0x01C => errwarn: ReadWrite<u32, ERRWARN::Register>),
        /// 0x020: Target DMA Control (not used)
        (0x020 => dmactrl: ReadWrite<u32>),
        /// 0x024-0x028: Reserved
        (0x024 => _reserved1: [u8; 0x08]),
        /// 0x02C: Target Data Control
        (0x02C => datactrl: ReadWrite<u32, DATACTRL::Register>),
        /// 0x030: Target Write Byte Data
        (0x030 => wdatab: ReadWrite<u32, WDATAB::Register>),
        /// 0x034: Target Write Byte Data as End
        (0x034 => wdatabe: ReadWrite<u32, WDATABE::Register>),
        /// 0x038: Target Write Half-Word Data
        (0x038 => wdatah: ReadWrite<u32>),
        /// 0x03C: Target Write Half-Word Data as End
        (0x03C => wdatahe: ReadWrite<u32>),
        /// 0x040: Target Read Byte Data
        (0x040 => rdatab: ReadOnly<u32, RDATAB::Register>),
        /// 0x044: Reserved
        (0x044 => _reserved2: [u8; 0x04]),
        /// 0x048: Target Read Half-Word Data
        (0x048 => rdatah: ReadOnly<u32>),
        /// 0x04C-0x053: Reserved
        (0x04C => _reserved3: [u8; 0x08]),
        /// 0x054-0x05F: Reserved (includes unused wdatab1 at 0x054)
        (0x054 => _reserved4: [u8; 0x0C]),
        /// 0x060: Target Capabilities
        (0x060 => capabilities: ReadOnly<u32, CAPABILITIES::Register>),
        /// 0x064: Target Dynamic Address
        (0x064 => dynaddr: ReadWrite<u32, DYNADDR::Register>),
        /// 0x068: Target Maximum Limits
        (0x068 => maxlimits: ReadWrite<u32, MAXLIMITS::Register>),
        /// 0x06C: Target Part Number
        (0x06C => partno: ReadWrite<u32, PARTNO::Register>),
        /// 0x070: Target ID Extension
        (0x070 => idext: ReadWrite<u32, IDEXT::Register>),
        /// 0x074: Target Vendor ID
        (0x074 => vendorid: ReadWrite<u32, VENDORID::Register>),
        /// 0x078: Target Timing Control Clock
        (0x078 => tcclock: ReadWrite<u32>),
        /// 0x07C-0x140: Reserved and controller-specific registers
        (0x07C => _reserved5: [u8; 0x14]),
        /// 0x90: Master Interrupt Enable Set Register (not used in target mode)
        (0x090 => mintset: ReadWrite<u32, MINTSET::Register>),
        (0x094 => _reserved6: [u8; 0xAC]),
        /// 0x140: Target Extended IBI Data Register 1
        (0x140 => ibiext1: ReadWrite<u32>),
        /// 0x144: Target Extended IBI Data Register 2
        (0x144 => ibiext2: ReadWrite<u32>),
        /// 0x148: End of registers
        (0x148 => @END),
    }
}

register_structs! {
    /// NPCM PDMA (Peripheral DMA) Descriptor Register
    PdmaDsctReg {
        /// 0x00: Control register
        (0x00 => ctl: ReadWrite<u32, PDMA_CTL::Register>),
        /// 0x04: Source address
        (0x04 => sa: ReadWrite<u32>),
        /// 0x08: Destination address
        (0x08 => da: ReadWrite<u32>),
        /// 0x0C: Next descriptor address
        (0x0C => next: ReadWrite<u32>),
        (0x10 => @END),
    }
}

register_structs! {
    /// NPCM PDMA Register Map
    PdmaRegisters {
        /// 0x000-0x0DC: Descriptor Table Control Registers 0-13
        (0x000 => dsct: [PdmaDsctReg; 14]),
        /// 0x0E0-0x0FC: Reserved
        (0x0E0 => _reserved1: [u8; 0x20]),
        /// 0x100-0x134: Current Scatter-Gather Descriptor Table Address 0-13
        (0x100 => curscat: [ReadOnly<u32>; 14]),
        /// 0x138-0x3FC: Reserved
        (0x138 => _reserved2: [u8; 0x2C8]),
        /// 0x400: PDMA Channel Control Register
        (0x400 => chctl: ReadWrite<u32, PDMA_CHCTL::Register>),
        /// 0x404: PDMA Stop Transfer Register
        (0x404 => stop: ReadWrite<u32>),
        /// 0x408: PDMA Software Request Register
        (0x408 => swreq: ReadWrite<u32>),
        /// 0x40C: PDMA Request Active Flag Register
        (0x40C => trgsts: ReadOnly<u32>),
        /// 0x410: PDMA Fixed Priority Setting Register
        (0x410 => priset: ReadWrite<u32>),
        /// 0x414: PDMA Fixed Priority Clear Register
        (0x414 => priclr: ReadWrite<u32>),
        /// 0x418: PDMA Interrupt Enable Control Register
        (0x418 => inten: ReadWrite<u32>),
        /// 0x41C: PDMA Interrupt Status Register
        (0x41C => intsts: ReadWrite<u32>),
        /// 0x420: PDMA Read/Write Target Abort Flag Register
        (0x420 => abtsts: ReadWrite<u32>),
        /// 0x424: PDMA Transfer Done Flag Register
        (0x424 => tdsts: ReadWrite<u32>),
        /// 0x428: PDMA Scatter-Gather Transfer Done Flag Register
        (0x428 => scatsts: ReadWrite<u32>),
        /// 0x42C: PDMA Transfer on Active Flag Register
        (0x42C => tactsts: ReadOnly<u32>),
        /// 0x430-0x438: Reserved
        (0x430 => _reserved3: [u8; 0x0C]),
        /// 0x43C: PDMA Scatter-Gather Descriptor Table Base Address Register
        (0x43C => scatba: ReadWrite<u32>),
        /// 0x440-0x47C: Reserved
        (0x440 => _reserved4: [u8; 0x40]),
        /// 0x480-0x48C: PDMA Source Module Select Register 0-3
        (0x480 => reqsel: [ReadWrite<u32>; 4]),
        (0x490 => @END),
    }
}

register_bitfields![u32,
    /// Controller Configuration Register (MCONFIG)
    /// Bit 0-1: MSTENA (0=Target mode, 1=Controller mode, 2=Secondary controller)
    MCONFIG [
        MSTENA OFFSET(0) NUMBITS(2) []
    ],

    /// Target Configuration Register (CONFIG)
    CONFIG [
        /// Static Address (bits 31-25)
        SADDR OFFSET(25) NUMBITS(7) [],
        /// Bus Available Matching (bits 22-16)
        BAMATCH OFFSET(16) NUMBITS(7) [],
        /// HDR Command (bit 10)
        HDRCMD OFFSET(10) NUMBITS(1) [],
        /// Offline mode (bit 9)
        OFFLINE OFFSET(9) NUMBITS(1) [],
        /// DDR mode support (bit 4)
        DDROK OFFSET(4) NUMBITS(1) [],
        /// Match Start and Stop (bit 2)
        MATCHSS OFFSET(2) NUMBITS(1) [],
        /// NACK all CCC (bit 1)
        NACK OFFSET(1) NUMBITS(1) [],
        /// Slave Enable (bits 0-1): 0=disabled, 1=enabled
        SLVENA OFFSET(0) NUMBITS(1) [
            Disabled = 0,
            Enabled = 1
        ]
    ],

    /// Target Status Register (STATUS)
    STATUS [
        /// Hot-join disabled
        HJDIS OFFSET(27) NUMBITS(1) [],
        /// IBI disabled
        IBIDIS OFFSET(24) NUMBITS(1) [],
        /// Event detected bits
        EVDET OFFSET(20) NUMBITS(2) [],
        /// Target reset detected (Secondary controller becomes target)
        SLVSTART OFFSET(19) NUMBITS(1) [],
        /// Event requested
        EVENT OFFSET(18) NUMBITS(1) [],
        /// CCC handled
        CHANDLED OFFSET(17) NUMBITS(1) [],
        /// DDR Match
        DDRMATCH OFFSET(16) NUMBITS(1) [],
        /// Error/Warning
        ERRWARN OFFSET(15) NUMBITS(1) [],
        /// CCC (Common Command Code) received
        CCC OFFSET(14) NUMBITS(1) [],
        /// Dynamic address valid
        DACHG OFFSET(13) NUMBITS(1) [],
        /// TX FIFO not full
        TXNOTFULL OFFSET(12) NUMBITS(1) [],
        /// RX FIFO not empty
        RXPEND OFFSET(11) NUMBITS(1) [],
        /// Stop detected
        STOP OFFSET(10) NUMBITS(1) [],
        /// Address matched
        MATCHED OFFSET(9) NUMBITS(1) [],
        /// Start detected
        START OFFSET(8) NUMBITS(1) [],
        /// SDR Request Write
        STREQWR OFFSET(4) NUMBITS(1) [],
        /// SDR Request Read
        STREQRD OFFSET(3) NUMBITS(1) []
    ],

    /// Target Control Register (CTRL)
    CTRL [
        /// IBI data
        IBIDATA OFFSET(8) NUMBITS(1) [],
        /// Extended IBI data
        EXTDATA OFFSET(3) NUMBITS(1) [],
        /// Event (bits 1-0)
        EVENT OFFSET(0) NUMBITS(2) [
            None = 0,
            EmitIBI = 1,
            EmitControllerRequest = 2,
            EmitHotJoin = 3
        ]
    ],

    /// Interrupt Enable Set Register (INTSET)
    INTSET [
        SLVSTART OFFSET(19) NUMBITS(1) [],
        EVENT OFFSET(18) NUMBITS(1) [],
        CHANDLED OFFSET(17) NUMBITS(1) [],
        DDRMATCH OFFSET(16) NUMBITS(1) [],
        ERRWARN OFFSET(15) NUMBITS(1) [],
        CCC OFFSET(14) NUMBITS(1) [],
        DACHG OFFSET(13) NUMBITS(1) [],
        TXNOTFULL OFFSET(12) NUMBITS(1) [],
        RXPEND OFFSET(11) NUMBITS(1) [],
        STOP OFFSET(10) NUMBITS(1) [],
        MATCHED OFFSET(9) NUMBITS(1) [],
        START OFFSET(8) NUMBITS(1) []
    ],

    /// Master Interrupt Enable Set Register (MINTSET) - not used in target mode
    MINTSET [
        NOWMASTER OFFSET(19) NUMBITS(1) []
    ],

    /// Interrupt Enable Clear Register (INTCLR) - same fields as INTSET
    INTCLR [
        SLVSTART OFFSET(19) NUMBITS(1) [],
        EVENT OFFSET(18) NUMBITS(1) [],
        CHANDLED OFFSET(17) NUMBITS(1) [],
        DDRMATCH OFFSET(16) NUMBITS(1) [],
        ERRWARN OFFSET(15) NUMBITS(1) [],
        CCC OFFSET(14) NUMBITS(1) [],
        DACHG OFFSET(13) NUMBITS(1) [],
        TXNOTFULL OFFSET(12) NUMBITS(1) [],
        RXPEND OFFSET(11) NUMBITS(1) [],
        STOP OFFSET(10) NUMBITS(1) [],
        MATCHED OFFSET(9) NUMBITS(1) [],
        START OFFSET(8) NUMBITS(1) []
    ],

    /// Interrupt Masked Register (INTMASKED) - Read-only status
    INTMASKED [
        SLVSTART OFFSET(19) NUMBITS(1) [],
        EVENT OFFSET(18) NUMBITS(1) [],
        CHANDLED OFFSET(17) NUMBITS(1) [],
        DDRMATCH OFFSET(16) NUMBITS(1) [],
        ERRWARN OFFSET(15) NUMBITS(1) [],
        CCC OFFSET(14) NUMBITS(1) [],
        DACHG OFFSET(13) NUMBITS(1) [],
        TXNOTFULL OFFSET(12) NUMBITS(1) [],
        RXPEND OFFSET(11) NUMBITS(1) [],
        STOP OFFSET(10) NUMBITS(1) [],
        MATCHED OFFSET(9) NUMBITS(1) [],
        START OFFSET(8) NUMBITS(1) []
    ],

    /// Error and Warning Register (ERRWARN)
    ERRWARN [
        /// Overwrite error
        OWRITE OFFSET(17) NUMBITS(1) [],
        /// Overread error
        OREAD OFFSET(16) NUMBITS(1) [],
        /// NACK during HDR
        HCRC OFFSET(10) NUMBITS(1) [],
        /// Parity error
        HPAR OFFSET(9) NUMBITS(1) [],
        /// Underflow
        URUN OFFSET(1) NUMBITS(1) [],
        /// Overflow
        ORUN OFFSET(0) NUMBITS(1) []
    ],

    /// Data Control Register (DATACTRL)
    DATACTRL [
        /// RX FIFO empty
        RXEMPTY OFFSET(31) NUMBITS(1) [],
        /// TX FIFO full
        TXFULL OFFSET(30) NUMBITS(1) [],
        /// RX FIFO count (bits 28-24)
        RXCOUNT OFFSET(24) NUMBITS(5) [],
        /// TX FIFO count (bits 20-16)
        TXCOUNT OFFSET(16) NUMBITS(5) [],
        /// Flush TX buffer
        FLUSHTB OFFSET(0) NUMBITS(1) [],
        /// Flush RX buffer
        FLUSHFB OFFSET(1) NUMBITS(1) []
    ],

    /// Write Data Byte Register (WDATAB)
    WDATAB [
        /// End bit
        END OFFSET(8) NUMBITS(1) [],
        /// Data byte
        DATA OFFSET(0) NUMBITS(8) []
    ],

    /// Write Data Byte End Register (WDATABE)
    WDATABE [
        /// Data byte with implicit end
        DATA OFFSET(0) NUMBITS(8) []
    ],

    /// Read Data Byte Register (RDATAB)
    RDATAB [
        /// Data byte read from RX FIFO
        DATA OFFSET(0) NUMBITS(8) []
    ],

    /// Capabilities Register (CAPABILITIES)
    CAPABILITIES [
        /// DMA support
        DMA OFFSET(31) NUMBITS(1) [],
    ],

    /// Part Number Register (PARTNO) - Lower 32 bits of PID
    PARTNO [
        /// Part ID (bits 31-16)
        PARTID OFFSET(16) NUMBITS(16) [],
        /// Instance ID (bits 15-12)
        INSTANCEID OFFSET(12) NUMBITS(4) [],
        /// Vendor-defined information (bits 11-0)
        VENDORDEF OFFSET(0) NUMBITS(12) []
    ],

    /// ID Extension Register (IDEXT) - Contains Vendor ID
    IDEXT [
        /// Bus Characteristics Register (BCR) - bits 23-16
        BCR OFFSET(16) NUMBITS(8) [],
        /// Device Characteristics Register (DCR) - bits 15-8
        DCR OFFSET(8) NUMBITS(8) [],
        /// Vendor ID lower 8 bits (bits 7-0)
        VENDORID_LOW OFFSET(0) NUMBITS(8) []
    ],

    /// Vendor ID Register (VENDORID) - Upper part of Vendor ID
    VENDORID [
        /// Vendor ID upper 7 bits (bits 14-8 of vendor ID)
        VENDORID_HIGH OFFSET(0) NUMBITS(15) []
    ],

    /// Dynamic Address Register (DYNADDR)
    DYNADDR [
        /// Dynamic address valid
        DAVALID OFFSET(0) NUMBITS(1) [],
        /// Dynamic address value (bits 7-1)
        DADDR OFFSET(1) NUMBITS(7) []
    ],

    /// Maximum Limits Register (MAXLIMITS)
    MAXLIMITS [
        /// Maximum write length (bits 27-16)
        MAXWR OFFSET(16) NUMBITS(12) [],
        /// Maximum read length (bits 11-0)
        MAXRD OFFSET(0) NUMBITS(12) []
    ],

    /// PDMA Control Register (PDMA_CTL)
    PDMA_CTL [
        /// Transfer width: 00=8bit, 01=16bit, 10=32bit
        TXWIDTH OFFSET(12) NUMBITS(2) [
            Byte = 0,
            HalfWord = 1,
            Word = 2
        ],
        /// Burst size: number of transfers per request
        BURSIZE OFFSET(8) NUMBITS(3) [],
        /// DMA mode: 00=mem2mem, 01=periph2mem, 10=mem2periph
        MODE OFFSET(4) NUMBITS(2) [
            Mem2Mem = 0,
            Periph2Mem = 1,
            Mem2Periph = 2
        ],
        /// Table interrupt enable
        TIEN OFFSET(2) NUMBITS(1) [],
        /// Operation mode: 0=basic, 1=scatter-gather
        OPMODE OFFSET(1) NUMBITS(1) [
            Basic = 0,
            ScatterGather = 1
        ]
    ],

    /// PDMA Channel Control Register (PDMA_CHCTL)
    PDMA_CHCTL [
        /// Channel 13 enable
        CH13EN OFFSET(13) NUMBITS(1) [],
        /// Channel 12 enable
        CH12EN OFFSET(12) NUMBITS(1) [],
        /// Channel 11 enable (I3C6 TX)
        CH11EN OFFSET(11) NUMBITS(1) [],
        /// Channel 10 enable (I3C6 RX)
        CH10EN OFFSET(10) NUMBITS(1) [],
        /// Channel 9 enable (I3C5 TX)
        CH9EN OFFSET(9) NUMBITS(1) [],
        /// Channel 8 enable (I3C5 RX)
        CH8EN OFFSET(8) NUMBITS(1) [],
        /// Channel 7 enable (I3C4 TX)
        CH7EN OFFSET(7) NUMBITS(1) [],
        /// Channel 6 enable (I3C4 RX)
        CH6EN OFFSET(6) NUMBITS(1) [],
        /// Channel 5 enable (I3C3 TX)
        CH5EN OFFSET(5) NUMBITS(1) [],
        /// Channel 4 enable (I3C3 RX)
        CH4EN OFFSET(4) NUMBITS(1) [],
        /// Channel 3 enable (I3C2 TX)
        CH3EN OFFSET(3) NUMBITS(1) [],
        /// Channel 2 enable (I3C2 RX)
        CH2EN OFFSET(2) NUMBITS(1) [],
        /// Channel 1 enable (I3C1 TX)
        CH1EN OFFSET(1) NUMBITS(1) [],
        /// Channel 0 enable (I3C1 RX)
        CH0EN OFFSET(0) NUMBITS(1) []
    ]
];

/// I3C operation state
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum OperState {
    Idle,
    Write,
    Read,
    Ibi,
}

/// I3C Target Driver Client trait
///
/// This trait defines the callbacks for I3C target mode operations.
///
/// ## Buffer Mode Support (Future Enhancement)
///
/// The current implementation uses a simple callback model where the client is notified
/// when data is ready. For compatibility with Zephyr's buffer mode and more advanced
/// use cases, the following extensions could be added:
///
/// ### Buffer Mode Extension (not yet implemented):
/// ```ignore
/// /// Called when a buffer-based read is requested (target should provide buffer)
/// fn buf_read_requested(&self, buf_ptr: &mut Option<&'static mut [u8]>,
///                       buf_len: &mut Option<usize>,
///                       buf_offset: &mut Option<usize>) {
///     // Client provides a buffer to be transmitted
///     // This allows zero-copy operation for large transfers
/// }
///
/// /// Called when a buffer-based write is received (provides buffer info)
/// fn buf_write_received(&self, buffer: &'static mut [u8],
///                       len: usize,
///                       error: Result<(), ErrorCode>) {
///     // Similar to write_complete but optimized for buffer mode
/// }
/// ```
///
/// ### Benefits of Buffer Mode:
/// - **Zero-copy operation**: Data can be DMA'd directly to/from application buffers
/// - **Reduced latency**: No intermediate buffering required
/// - **Better performance**: Especially for large transfers
/// - **Zephyr compatibility**: Matches Zephyr's I3C target API design
///
/// ### Implementation Notes:
/// To add buffer mode support:
/// 1. Extend this trait with `buf_read_requested` and `buf_write_received` methods
/// 2. Add a configuration option to select between callback and buffer modes
/// 3. Modify `handle_matched` to call buffer mode callbacks when enabled
/// 4. Update DMA setup to use application-provided buffers directly
///
pub trait Client {
    /// Called when a write transfer is complete
    fn write_complete(&self, buffer: &'static mut [u8], len: usize, error: Result<(), ErrorCode>);

    /// Called when controller requests a read (target should provide data)
    fn read_requested(&self);

    /// Called when an IBI is complete
    fn ibi_complete(&self, error: Result<(), ErrorCode>);

    /// Called when hot-join request is complete
    fn hotjoin_complete(&self, error: Result<(), ErrorCode>);
}

/// Transfer mode
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum TransferMode {
    /// FIFO mode (software polling)
    Fifo,
    /// DMA mode (hardware-assisted)
    Dma,
}

/// NPCM I3C Target Driver
pub struct I3cTarget<'a> {
    registers: StaticRef<I3cRegisters>,
    dma_registers: StaticRef<PdmaRegisters>,

    /// Current operation state
    state: Cell<OperState>,

    /// Transfer mode (FIFO or DMA)
    transfer_mode: Cell<TransferMode>,

    /// RX buffer for incoming writes
    rx_buffer: TakeCell<'static, [u8]>,
    rx_len: Cell<usize>,

    /// TX buffer for outgoing reads
    tx_buffer: TakeCell<'static, [u8]>,
    tx_len: Cell<usize>,
    tx_idx: Cell<usize>,

    /// DMA channels
    dma_rx_channel: u8,
    dma_tx_channel: u8,

    /// Client callback (native npcm400 Client trait)
    client: OptionalCell<&'a dyn Client>,

    /// HIL RX client for MCTP compatibility
    hil_rx_client: OptionalCell<&'a dyn hil::RxClient>,

    /// HIL TX client for MCTP compatibility
    hil_tx_client: OptionalCell<&'a dyn hil::TxClient>,

    /// Target configuration
    static_address: Cell<u8>,
    dynamic_address: Cell<Option<u8>>,
    max_read_len: Cell<u16>,
    max_write_len: Cell<u16>,

    /// Device identification
    pid: Cell<ProvisionedId>,
    bcr: Cell<BusCharacteristics>,
    dcr: Cell<DeviceCharacteristics>,
}

impl<'a> I3cTarget<'a> {
    /// Create I3C1 instance
    pub const fn new_i3c1() -> Self {
        Self::new_with_params(I3C1_BASE, I3C1_DMA_RX_CHANNEL, I3C1_DMA_TX_CHANNEL)
    }

    /// Create I3C2 instance
    pub const fn new_i3c2() -> Self {
        Self::new_with_params(I3C2_BASE, I3C2_DMA_RX_CHANNEL, I3C2_DMA_TX_CHANNEL)
    }

    /// Create I3C3 instance
    pub const fn new_i3c3() -> Self {
        Self::new_with_params(I3C3_BASE, I3C3_DMA_RX_CHANNEL, I3C3_DMA_TX_CHANNEL)
    }

    /// Create I3C4 instance
    pub const fn new_i3c4() -> Self {
        Self::new_with_params(I3C4_BASE, I3C4_DMA_RX_CHANNEL, I3C4_DMA_TX_CHANNEL)
    }

    /// Create I3C5 instance
    pub const fn new_i3c5() -> Self {
        Self::new_with_params(I3C5_BASE, I3C5_DMA_RX_CHANNEL, I3C5_DMA_TX_CHANNEL)
    }

    /// Create I3C6 instance
    pub const fn new_i3c6() -> Self {
        Self::new_with_params(I3C6_BASE, I3C6_DMA_RX_CHANNEL, I3C6_DMA_TX_CHANNEL)
    }

    /// Internal constructor with parameters
    const fn new_with_params(
        registers: StaticRef<I3cRegisters>,
        dma_rx_channel: u8,
        dma_tx_channel: u8,
    ) -> Self {
        // Default PID = 0x0632_12344567 (48 bits)
        // Vendor=0x0319, Part=0x1234, Instance=0x4, Extra=0x567
        let default_pid = ProvisionedId::new(0x0319, 0x1234, 0x4, 0x567);

        Self {
            registers,
            dma_registers: PDMA_BASE,
            state: Cell::new(OperState::Idle),
            transfer_mode: Cell::new(TransferMode::Fifo), // Default to Fifo mode
            rx_buffer: TakeCell::empty(),
            rx_len: Cell::new(0),
            tx_buffer: TakeCell::empty(),
            tx_len: Cell::new(0),
            tx_idx: Cell::new(0),
            dma_rx_channel,
            dma_tx_channel,
            client: OptionalCell::empty(),
            hil_rx_client: OptionalCell::empty(),
            hil_tx_client: OptionalCell::empty(),
            static_address: Cell::new(0x3a), // Default address
            dynamic_address: Cell::new(None),
            max_read_len: Cell::new(256),
            max_write_len: Cell::new(256),
            pid: Cell::new(default_pid),
            bcr: Cell::new(BusCharacteristics::new_target()),
            dcr: Cell::new(DeviceCharacteristics::DEFAULT),
        }
    }

    /// Legacy constructor for I3C1 (for backwards compatibility)
    pub const fn new() -> Self {
        Self::new_i3c1()
    }

    /// Enable DMA mode for high-speed transfers
    pub fn enable_dma(&self) {
        self.transfer_mode.set(TransferMode::Dma);
        self.init_dma();
    }

    /// Disable DMA mode and use FIFO mode
    pub fn disable_dma(&self) {
        self.transfer_mode.set(TransferMode::Fifo);
        self.deinit_dma();
    }

    /// Debug GPIOs
    pub fn enable_debug_gpio86_87(&self) {
        unsafe {
            // GPIO86
            *(0x400C_3011_i32 as *mut u8) &= !0x80_u8; // mux: b7
            *(0x4009_1000_i32 as *mut u8) &= !0x40_u8; // val: b6
            *(0x4009_1002_i32 as *mut u8) |= 0x40_u8; // dir: b6

            // GPIO87
            *(0x400C_3011_i32 as *mut u8) &= !0x20_u8; // mux: b5
            *(0x4009_1000_i32 as *mut u8) &= !0x80_u8; // val: b7
            *(0x4009_1002_i32 as *mut u8) |= 0x80_u8; // dir: b7
        }
    }

    pub fn set_debug_gpio86(&self, high: bool) {
        unsafe {
            if high {
                *(0x4009_1000_i32 as *mut u8) |= 0x40_u8; // Set GPIO86 high
            } else {
                *(0x4009_1000_i32 as *mut u8) &= !0x40_u8; // Set GPIO86 low
            }
        }
    }

    pub fn set_debug_gpio87(&self, high: bool) {
        unsafe {
            if high {
                *(0x4009_1000_i32 as *mut u8) |= 0x80_u8; // Set GPIO87 high
            } else {
                *(0x4009_1000_i32 as *mut u8) &= !0x80_u8; // Set GPIO87 low
            }
        }
    }

    /// Initialize DMA controller for I3C transfers
    fn init_dma(&self) {
        let dma = self.dma_registers;

        // Enable DMA channels for this I3C instance
        let rx_bit = 1 << self.dma_rx_channel;
        let tx_bit = 1 << self.dma_tx_channel;
        dma.chctl.set(dma.chctl.get() | rx_bit | tx_bit);

        // Configure RX channel (Periph to Mem)
        let rx_desc = &dma.dsct[self.dma_rx_channel as usize];
        rx_desc.ctl.set(
            (PDMA_CTL::MODE::Periph2Mem
                + PDMA_CTL::TXWIDTH::Byte
                + PDMA_CTL::OPMODE::Basic
                + PDMA_CTL::BURSIZE.val(0))
            .value, // Single transfer
        );
        rx_desc.sa.set(&self.registers.rdatab as *const _ as u32); // I3C RX FIFO

        // Configure TX channel (Mem to Periph)
        let tx_desc = &dma.dsct[self.dma_tx_channel as usize];
        tx_desc.ctl.set(
            (PDMA_CTL::MODE::Mem2Periph
                + PDMA_CTL::TXWIDTH::Byte
                + PDMA_CTL::OPMODE::Basic
                + PDMA_CTL::BURSIZE.val(0))
            .value, // Single transfer
        );
        tx_desc.da.set(&self.registers.wdatab as *const _ as u32); // I3C TX FIFO

        // Enable DMA interrupts for both channels
        let rx_mask = 1 << self.dma_rx_channel;
        let tx_mask = 1 << self.dma_tx_channel;
        dma.inten.set(dma.inten.get() | rx_mask | tx_mask);

        // Configure I3C DMA control
        self.registers.dmactrl.set(
            (1 << 0) | // Enable RX DMA
            (1 << 1), // Enable TX DMA
        );
    }

    /// Deinitialize DMA controller
    fn deinit_dma(&self) {
        let dma = self.dma_registers;

        // Disable DMA channels for this I3C instance
        let rx_bit = 1 << self.dma_rx_channel;
        let tx_bit = 1 << self.dma_tx_channel;
        dma.chctl.set(dma.chctl.get() & !(rx_bit | tx_bit));

        // Disable I3C DMA control
        self.registers.dmactrl.set(0);

        // Disable DMA interrupts
        let rx_mask = 1 << self.dma_rx_channel;
        let tx_mask = 1 << self.dma_tx_channel;
        dma.inten.set(dma.inten.get() & !(rx_mask | tx_mask));
    }

    /// Set the client for callbacks
    pub fn set_client(&self, client: &'a dyn Client) {
        self.client.set(client);
    }

    /// Set Provisioned ID (PID)
    pub fn set_pid(&self, pid: ProvisionedId) {
        self.pid.set(pid);
    }

    /// Get Provisioned ID (PID)
    pub fn get_pid(&self) -> ProvisionedId {
        self.pid.get()
    }

    /// Set Bus Characteristics Register (BCR)
    pub fn set_bcr(&self, bcr: BusCharacteristics) {
        self.bcr.set(bcr);
    }

    /// Get Bus Characteristics Register (BCR)
    pub fn get_bcr(&self) -> BusCharacteristics {
        self.bcr.get()
    }

    /// Set Device Characteristics Register (DCR)
    pub fn set_dcr(&self, dcr: DeviceCharacteristics) {
        self.dcr.set(dcr);
    }

    /// Get Device Characteristics Register (DCR)
    pub fn get_dcr(&self) -> DeviceCharacteristics {
        self.dcr.get()
    }

    /// Configure PID with individual components
    pub fn configure_pid(&self, vendor_id: u16, part_id: u16, instance_id: u8, extra_info: u16) {
        let pid = ProvisionedId::new(vendor_id, part_id, instance_id, extra_info);
        self.set_pid(pid);
    }

    /// Reset the I3C module
    ///
    /// This function performs a software reset of the I3C module by:
    /// 1. Setting the corresponding bit in PMC SW_RST1 register
    /// 2. Waiting one NOP cycle
    /// 3. Clearing the bit to complete the reset
    pub fn reset_module(&self) {
        use core::arch::asm;

        // Get the hardware index (0-5 for I3C1-I3C6) based on the register pointer
        // Compare the StaticRef addresses
        let index = if core::ptr::eq(&*self.registers, &*I3C1_BASE) {
            0 // I3C1
        } else if core::ptr::eq(&*self.registers, &*I3C2_BASE) {
            1 // I3C2
        } else if core::ptr::eq(&*self.registers, &*I3C3_BASE) {
            2 // I3C3
        } else if core::ptr::eq(&*self.registers, &*I3C4_BASE) {
            3 // I3C4
        } else if core::ptr::eq(&*self.registers, &*I3C5_BASE) {
            4 // I3C5
        } else if core::ptr::eq(&*self.registers, &*I3C6_BASE) {
            5 // I3C6
        } else {
            i3c_debug!("I3C reset_module: Unknown I3C instance");
            return;
        };

        // PMC base address and SW_RST1 offset
        const PMC_BASE: u32 = 0x4000_D000;
        const SW_RST1_OFFSET: u32 = 0x13;
        let sw_rst1_addr = (PMC_BASE + SW_RST1_OFFSET) as *mut u8;

        unsafe {
            // Set the reset bit (write 1)
            let current_val = core::ptr::read_volatile(sw_rst1_addr);
            core::ptr::write_volatile(sw_rst1_addr, current_val | (1 << index));

            // Require one NOP instruction cycle time
            asm!("nop");

            // Clear the reset bit (write 0)
            let current_val = core::ptr::read_volatile(sw_rst1_addr);
            core::ptr::write_volatile(sw_rst1_addr, current_val & !(1 << index));
        }
    }

    /// Initialize the I3C target controller
    pub fn init(&self, static_addr: u8) {
        // Reset the I3C module first
        self.reset_module();

        let regs = self.registers;

        // Store static address
        self.static_address.set(static_addr);

        // Ensure controller mode is disabled (target mode)
        regs.mconfig.set(MCONFIG::MSTENA.val(0).value);

        // Disable slave mode first to configure registers safely
        regs.config.modify(CONFIG::SLVENA::Disabled);

        // Clear all interrupts and disable them during configuration
        regs.intclr.set(0xFFFFFFFF);
        regs.status.set(0xFFFFFFFF); // Clear all status bits

        // Configure PID (Provisioned ID)
        let pid = self.pid.get();

        // Write PARTNO register (lower 32 bits of PID)
        let partno_value = ((pid.part_id as u32) << 16)
            | ((pid.instance_id as u32) << 12)
            | (pid.extra_info as u32);
        regs.partno.set(partno_value);

        // Configure Vendor ID and characteristics
        let bcr = self.bcr.get();
        let dcr = self.dcr.get();
        regs.idext.set(
            (IDEXT::DCR.val(dcr.to_u8() as u32)
                + IDEXT::BCR.val(bcr.to_u8() as u32)
                + IDEXT::VENDORID_LOW.val((pid.vendor_id & 0xFF) as u32))
            .value,
        );
        regs.vendorid
            .set(VENDORID::VENDORID_HIGH.val(pid.get_pid_high()).value);

        // Set maximum read/write lengths
        regs.maxlimits.set(
            (MAXLIMITS::MAXRD.val(self.max_read_len.get() as u32)
                + MAXLIMITS::MAXWR.val(self.max_write_len.get() as u32))
            .value,
        );

        // Set hdrcmd to 0 (written to the receive buffer/FIFO)
        regs.config.modify(CONFIG::HDRCMD::CLEAR);

        // Clear any pending errors
        regs.errwarn.set(0xFFFFFFFF);

        // Configure target with static address
        regs.config
            .set((CONFIG::SADDR.val(static_addr as u32) + CONFIG::SLVENA::Disabled).value);

        // Calculate bamatch: frequency rate in MHz (rounded up)
        let i3c_freq_rate = 96_000_000; // TODO: Assume 96 MHz, need to get actual clock rate
        let bamatch = (i3c_freq_rate + 1_000_000 - 1) / 1_000_000;
        regs.config.modify(CONFIG::BAMATCH.val(bamatch));

        // Disable Match Start/Stop
        regs.config.modify(CONFIG::MATCHSS::CLEAR);

        // Clear all interrupts
        regs.intclr.set(regs.intset.get());

        // Clear all status bits
        regs.status.set(regs.status.get());

        // Enable interrupts:
        let intset_mask = (INTSET::START::SET
            + INTSET::MATCHED::SET
            + INTSET::RXPEND::SET     // Enable RXPEND to receive write data
            + INTSET::STOP::SET
            + INTSET::DACHG::SET
            + INTSET::CCC::SET
            + INTSET::ERRWARN::SET
            + INTSET::DDRMATCH::SET
            + INTSET::CHANDLED::SET
            + INTSET::EVENT::SET)
            .value;
        regs.intset.set(intset_mask);

        let mintset_mask = MINTSET::NOWMASTER::SET.value;
        regs.mintset.set(mintset_mask);

        // Finally, enable slave mode after everything is configured
        regs.config.modify(CONFIG::SLVENA::Enabled);

        // Flush FIFOs
        regs.datactrl
            .modify(DATACTRL::FLUSHTB::SET + DATACTRL::FLUSHFB::SET);

        self.enable_debug_gpio86_87();
    }

    /// Handle I3C interrupt
    /// This function reads the interrupt status, identifies the source of the interrupt,
    /// and calls the appropriate handler for each interrupt type.
    /// When the interrupt is not handled, the tock kernel may hang or misbehave.
    ///
    /// Implementation follows Zephyr's approach: loop until all interrupts are cleared
    /// to handle cases where new interrupts arrive during processing.
    pub fn handle_interrupt(&self) {
        let regs = self.registers;
        let mut int_masked = regs.intmasked.get();

        // Return early if no interrupts (spurious interrupt)
        if int_masked == 0 {
            // Re-read INTMASKED in case of race condition
            int_masked = regs.intmasked.get();
            if int_masked == 0 {
                i3c_debug!("I3C Target Spurious Interrupt");
                return;
            }
        }

        self.set_debug_gpio87(true);

        // Loop until all interrupts are processed
        // This ensures new interrupts that arrive during processing are also handled
        while int_masked != 0 {
            // Handle errors first
            if (int_masked & INTMASKED::ERRWARN::SET.value) != 0 {
                self.handle_error();
            }

            // Handle START condition (includes repeated start)
            if (int_masked & INTMASKED::START::SET.value) != 0 {
                self.handle_start();

                // Clear START interrupt (W1C)
                regs.status.set(STATUS::START::SET.value);
            }

            // Handle SLVSTART (target reset detection)
            if (int_masked & INTMASKED::SLVSTART::SET.value) != 0 {
                self.handle_slvstart();

                // Clear SLVSTART interrupt (W1C)
                regs.status.set(STATUS::SLVSTART::SET.value);
            }

            // Handle address MATCHED
            // IMPORTANT: Process MATCHED BEFORE RXPEND so state is set correctly (Write vs Read)
            // before we try to read data from FIFO
            if (int_masked & INTMASKED::MATCHED::SET.value) != 0 {
                self.handle_matched();

                // If CONFIG.MATCHSS=1, MATCHED bit must remain 1 to detect next start or stop.
                // Clear the status bit in STOP or START handler.
                let config = regs.config.get();
                if (config & CONFIG::MATCHSS::SET.value) != 0 {
                    regs.intclr.modify(INTCLR::MATCHED::SET);
                } else {
                    // W1C
                    regs.status.set(STATUS::MATCHED::SET.value);
                }
            }

            // Handle RX data pending (write from controller)
            // IMPORTANT: Process RXPEND AFTER MATCHED (so state is set) but BEFORE STOP
            // (to read all data before completing transfer)
            if (int_masked & INTMASKED::RXPEND::SET.value) != 0 {
                self.handle_rxpend();
                // RXPEND auto-clears when FIFO is empty
            }

            // Handle STOP condition
            // Process AFTER RXPEND to ensure all data is read before completing transfer
            if (int_masked & INTMASKED::STOP::SET.value) != 0 {
                self.handle_stop();

                // Clear STOP interrupt
                regs.status.set(STATUS::STOP::SET.value);
            }

            // Handle dynamic address change
            if (int_masked & INTMASKED::DACHG::SET.value) != 0 {
                self.handle_dachg();

                // Clear DACHG interrupt (W1C)
                regs.status.set(STATUS::DACHG::SET.value);
            }

            // Handle TX not full (read from controller)
            if (int_masked & INTMASKED::TXNOTFULL::SET.value) != 0 {
                // Clear TXNOTFULL interrupt (W1C)
                regs.status.set(STATUS::TXNOTFULL::SET.value);
            }

            // Handle CCC (Common Command Code)
            if (int_masked & INTMASKED::CCC::SET.value) != 0 {
                // Clear CCC interrupt (W1C)
                regs.status.set(STATUS::CCC::SET.value);
            }

            // Handle DDRMATCH
            if (int_masked & INTMASKED::DDRMATCH::SET.value) != 0 {
                // Clear DDRMATCH interrupt (W1C)
                regs.status.set(STATUS::DDRMATCH::SET.value);
            }

            // Handle CHANDLED
            if (int_masked & INTMASKED::CHANDLED::SET.value) != 0 {
                // Flush FIFOs, the cmd code will remain in the buffer
                regs.datactrl.modify(DATACTRL::FLUSHFB::SET);
                regs.datactrl.modify(DATACTRL::FLUSHTB::SET);

                // Clear CHANDLED interrupt (W1C)
                regs.status.set(STATUS::CHANDLED::SET.value);
            }

            // Handle EVENT (IBI, hot-join, controller role request)
            if (int_masked & INTMASKED::EVENT::SET.value) != 0 {
                self.handle_event();

                // Clear EVENT interrupt (W1C)
                regs.status.set(STATUS::EVENT::SET.value);
            }

            // Re-check interrupts to handle any new interrupts that arrived during processing
            int_masked = regs.intmasked.get();
        }

        self.set_debug_gpio87(false);
    }

    /// Handle error condition
    fn handle_error(&self) {
        let regs = self.registers;
        let err = regs.errwarn.get();

        i3c_debug!("I3C Target Error: ERRWARN=0x{:X}", err);

        // Clear errors by writing 1s
        regs.errwarn.set(err);
    }

    /// Handle RXPEND interrupt (RX data pending - write from controller)
    ///
    /// This method handles incoming data from the controller. It reads data from the
    /// RX FIFO into the provided buffer. If no buffer is available or we're in an
    /// unexpected state, it flushes the FIFO to prevent continuous interrupts.
    fn handle_rxpend(&self) {
        let regs = self.registers;

        // Read data from RX FIFO
        if self.state.get() == OperState::Write {
            // Check if we have a buffer available
            if self.rx_buffer.is_some() {
                self.rx_buffer.map(|buffer| {
                    let mut len = self.rx_len.get();

                    // Read available data from RX FIFO
                    if self.transfer_mode.get() == TransferMode::Fifo {
                        while regs.status.is_set(STATUS::RXPEND) && len < buffer.len() {
                            buffer[len] = regs.rdatab.read(RDATAB::DATA) as u8;
                            len += 1;
                        }
                    }

                    self.rx_len.set(len);
                });
            } else {
                // No buffer available - must flush FIFO to prevent continuous interrupts
                if self.transfer_mode.get() == TransferMode::Fifo {
                    regs.datactrl.modify(DATACTRL::FLUSHFB::SET);
                }
            }
        } else {
            // Not in Write state - flush unexpected data
            if self.transfer_mode.get() == TransferMode::Fifo {
                regs.datactrl.modify(DATACTRL::FLUSHFB::SET);
            }
        }
    }

    /// Handle dynamic address change
    fn handle_dachg(&self) {
        let regs = self.registers;
        let dynaddr = regs.dynaddr.read(DYNADDR::DADDR);

        if regs.dynaddr.is_set(DYNADDR::DAVALID) {
            self.dynamic_address.set(Some(dynaddr as u8));
            i3c_debug!(
                "[I3C Target driver] Dynamic address assigned: 0x{:X}",
                dynaddr
            );
        } else {
            self.dynamic_address.set(None);
            i3c_debug!("[I3C Target driver] Dynamic address cleared");
        }
    }

    /// Handle SLVSTART (target reset detection)
    ///
    /// SLVSTART indicates that the target has been reset or that a secondary controller
    /// has transitioned back to target mode. This interrupt is primarily used in
    /// secondary controller scenarios where the device can switch between controller
    /// and target modes.
    ///
    /// When this interrupt occurs, the driver should:
    /// 1. Reset internal state
    /// 2. Prepare for new transactions
    /// 3. Optionally notify upper layers of the reset event
    fn handle_slvstart(&self) {
        i3c_debug!("[I3C Target driver] SLVSTART - Target reset detected");

        // Reset transfer state
        let state = self.state.get();
        if state != OperState::Idle {
            // If we were in the middle of a transfer, abort it
            match state {
                OperState::Write => {
                    // Abort write transfer
                    self.rx_buffer.take().map(|buffer| {
                        self.client.map(|client| {
                            client.write_complete(buffer, 0, Err(ErrorCode::CANCEL));
                        });
                    });
                    self.rx_len.set(0);
                }
                OperState::Read => {
                    // Abort read transfer
                    self.tx_buffer.take();
                    self.tx_len.set(0);
                    self.tx_idx.set(0);
                }
                OperState::Ibi => {
                    // Abort IBI
                    self.client.map(|client| {
                        client.ibi_complete(Err(ErrorCode::CANCEL));
                    });
                }
                _ => {}
            }

            self.state.set(OperState::Idle);
        }

        // Note: For secondary controller support, additional handling would be needed here
        // to manage the transition between controller and target modes.
    }

    /// Handle START condition (including repeated start)
    ///
    /// A START can occur in two scenarios:
    /// 1. New transaction: Just reset state for new transfer
    /// 2. Repeated Start (Sr): Complete the previous transfer and prepare for new one
    ///
    /// According to I3C spec, a repeated start terminates the previous transfer,
    /// so we need to complete any ongoing read/write operations.
    fn handle_start(&self) {
        let state = self.state.get();

        // If we're in an active transfer state, this is a repeated start (Sr)
        // Complete the previous transfer before starting new one
        match state {
            OperState::Write => {
                // Repeated start during write - complete the write transfer
                let len = self.rx_len.get();
                self.rx_buffer.take().map(|buffer| {
                    self.client.map(|client| {
                        client.write_complete(buffer, len, Ok(()));
                    });
                });
                self.rx_len.set(0);
                self.state.set(OperState::Idle);
            }
            OperState::Read => {
                // Repeated start during read - complete the read transfer
                if let Some(buffer) = self.tx_buffer.take() {
                    // Return TX buffer to client
                    if self.hil_tx_client.is_some() {
                        self.hil_tx_client.map(|client| {
                            client.send_done(buffer, Ok(()));
                        });
                    }
                }
                self.tx_len.set(0);
                self.tx_idx.set(0);
                self.state.set(OperState::Idle);
            }
            _ => {
                // Normal START - just reset counters
                self.rx_len.set(0);
                self.tx_idx.set(0);
            }
        }
    }

    /// Handle address MATCHED
    ///
    /// When the controller addresses us, we need to determine if it's a read or write request.
    /// For read requests in FIFO mode, we must immediately fill the TX FIFO because the
    /// controller will start clocking data immediately after the address phase.
    ///
    /// # Arguments
    /// * `status` - Snapshot of STATUS register captured at interrupt entry, containing
    ///              timing-sensitive STREQRD/STREQWR bits
    fn handle_matched(&self) {
        let regs = self.registers;

        // For pp=12.5MHz, wait one byte transfer time (1us) before checking for request type
        // Simple delay loop
        // TODO: Replace with timer-based delay if more accurate timing is needed
        for _ in 0..32 {
            core::hint::spin_loop();
        }

        let status = regs.status.get();

        // Check if it's a read or write by examining the captured status bits
        if (status & STATUS::RXPEND::SET.value) != 0 || (status & STATUS::STREQWR::SET.value) != 0 {
            // Write request from controller - target will receive data

            // If we have a pending TX operation (buffer waiting to be read),
            // cancel it and return the buffer to the client since the master
            // is sending a write instead of reading our response
            if self.tx_buffer.is_some() {
                if let Some(buffer) = self.tx_buffer.take() {
                    if self.hil_tx_client.is_some() {
                        self.hil_tx_client.map(|client| {
                            client.send_done(buffer, Err(ErrorCode::CANCEL));
                        });
                    }
                }
                self.tx_len.set(0);
                self.tx_idx.set(0);
            }

            // If no RX buffer is available, request one from the HIL client (MCTP)
            if self.rx_buffer.is_none() {
                self.hil_rx_client.map(|client| {
                    client.write_expected();
                });
            }

            // Change state to Write
            // The RXPEND interrupt handler will read the actual data from FIFO
            self.state.set(OperState::Write);
            self.rx_len.set(0); // Reset RX length counter

            // Flush TX FIFO to clear any stale response data from previous read
            // This prevents old data from being sent if master reads without us preparing a response
            let regs = self.registers;
            regs.datactrl.modify(DATACTRL::FLUSHTB::SET);
        } else {
            // Read request from controller - target must send data

            // Change state to Read
            self.state.set(OperState::Read);

            // In FIFO mode, we must immediately fill TX FIFO before handle_tx is called
            // because the controller will start clocking data right after address match.
            // This is critical for timing - it may be too late if we wait for handle_tx.
            if self.transfer_mode.get() == TransferMode::Fifo {
                self.tx_buffer.map(|buffer| {
                    let len = self.tx_len.get();
                    if len > 0 {
                        // Continue from where transmit() left off (tx_idx may already be set)
                        let mut idx = self.tx_idx.get();

                        // Fill TX FIFO with remaining data
                        while !regs.datactrl.is_set(DATACTRL::TXFULL) && idx < len {
                            if idx == len - 1 {
                                // Last byte - use WDATABE
                                regs.wdatabe
                                    .set(WDATABE::DATA.val(buffer[idx] as u32).value);
                            } else {
                                // Not last byte - use WDATAB without END
                                regs.wdatab.set(WDATAB::DATA.val(buffer[idx] as u32).value);
                            }
                            idx += 1;
                        }

                        self.tx_idx.set(idx);
                    }
                });
            }

            // If no buffer is available, request data from client
            if self.tx_buffer.is_none() {
                self.client.map(|client| client.read_requested());
            }
        }
    }

    /// Handle EVENT interrupt (IBI, hot-join, controller role request)
    /// For simplicity, we only handle IBI and hot-join here.
    fn handle_event(&self) {
        let regs = self.registers;
        let evdet = (regs.status.get() >> 10) & 0x3; // EVDET field

        // EVDET values:
        // 0 = No event or event not yet sent
        // 1 = Event request sent and acknowledged
        // 2 = Event request sent and NACKed
        // 3 = Event request unable to be sent
        match evdet {
            1 => {
                // Event acknowledged - notify client of success
                if self.state.get() == OperState::Ibi {
                    self.client.map(|client| {
                        client.hotjoin_complete(Ok(()));
                    });
                    self.state.set(OperState::Idle);
                }
            }
            2 | 3 => {
                // Event rejected or unable to send - notify client of failure
                if self.state.get() == OperState::Ibi {
                    self.client.map(|client| {
                        client.hotjoin_complete(Err(ErrorCode::FAIL));
                    });
                    self.state.set(OperState::Idle);
                }
            }
            _ => {
                // No event or not yet sent - do nothing
            }
        }
    }

    /// Handle RX data (write from controller to target)
    #[allow(dead_code)]
    fn handle_rx(&self) {
        let regs = self.registers;

        // Change state to Write
        self.state.set(OperState::Write);

        // Check transfer mode
        if self.transfer_mode.get() == TransferMode::Dma {
            // DMA mode - setup DMA transfer
            self.rx_buffer.map(|buffer| {
                self.setup_dma_rx(buffer, buffer.len());
            });
        } else {
            // FIFO mode - manual read
            self.rx_buffer.map(|buffer| {
                let mut len = self.rx_len.get();

                // Read while RX FIFO is not empty and buffer has space
                while !regs.datactrl.is_set(DATACTRL::RXEMPTY) && len < buffer.len() {
                    buffer[len] = regs.rdatab.read(RDATAB::DATA) as u8;
                    len += 1;
                }

                self.rx_len.set(len);
            });
        }
    }

    /// Setup DMA for RX transfer
    #[allow(dead_code)]
    fn setup_dma_rx(&self, buffer: &mut [u8], len: usize) {
        let dma = self.dma_registers;
        let desc = &dma.dsct[self.dma_rx_channel as usize];

        // Set destination address (buffer)
        desc.da.set(buffer.as_ptr() as u32);

        // Set transfer count
        // Note: DMA count is typically in the control register
        let count = len.min(4096); // Max DMA transfer size
        desc.ctl.modify(PDMA_CTL::BURSIZE.val((count as u32) & 0x7));

        // Trigger DMA transfer
        dma.swreq.set(1 << self.dma_rx_channel);
    }

    /// Handle TX (read from controller)
    fn handle_tx(&self) {
        let regs = self.registers;

        // If we don't have TX data, request it from client
        if self.tx_buffer.is_none() {
            self.state.set(OperState::Read);
            self.client.map(|client| client.read_requested());
            return;
        }

        // Check transfer mode
        if self.transfer_mode.get() == TransferMode::Dma {
            // DMA mode - setup DMA transfer
            self.tx_buffer.map(|buffer| {
                let len = self.tx_len.get();
                self.setup_dma_tx(buffer, len);
            });
        } else {
            // FIFO mode - manual write
            self.tx_buffer.map(|buffer| {
                let mut idx = self.tx_idx.get();
                let len = self.tx_len.get();

                // Write while TX FIFO is not full and we have data
                while !regs.datactrl.is_set(DATACTRL::TXFULL) && idx < len {
                    if idx == len - 1 {
                        // Last byte - use WDATABE
                        regs.wdatabe
                            .set(WDATABE::DATA.val(buffer[idx] as u32).value);
                    } else {
                        // Not last byte - use WDATAB without END
                        regs.wdatab.set(WDATAB::DATA.val(buffer[idx] as u32).value);
                    }
                    idx += 1;
                }
                self.tx_idx.set(idx);
            });
        }
    }

    /// Setup DMA for TX transfer
    fn setup_dma_tx(&self, buffer: &[u8], len: usize) {
        let dma = self.dma_registers;
        let desc = &dma.dsct[self.dma_tx_channel as usize];

        // Set source address (buffer)
        desc.sa.set(buffer.as_ptr() as u32);

        // Set transfer count
        let count = len.min(4096); // Max DMA transfer size
        desc.ctl.modify(PDMA_CTL::BURSIZE.val((count as u32) & 0x7));

        // Trigger DMA transfer
        dma.swreq.set(1 << self.dma_tx_channel);
    }

    /// Handle STOP condition
    ///
    /// STOP condition indicates the end of a transfer on the I3C bus.
    /// According to the hardware behavior, START and STOP can occur simultaneously
    /// (race condition), so we need to check for and clear any concurrent START interrupt.
    fn handle_stop(&self) {
        let regs = self.registers;
        let state = self.state.get();

        // Check for concurrent START interrupt
        // If START is also pending, clear it to avoid processing it separately
        // This can happen in certain timing conditions on the bus
        let int_masked = regs.intmasked.get();
        if (int_masked & INTMASKED::START::SET.value) != 0 {
            // Clear the START status bit (W1C)
            regs.status.set(STATUS::START::SET.value);
        }

        self.set_debug_gpio86(true);

        // Return to idle state BEFORE calling client callbacks
        // This allows clients (e.g., MCTP) to immediately queue TX data in response
        self.state.set(OperState::Idle);

        // Process the transfer completion based on previous state
        match state {
            OperState::Write => {
                // Write complete - notify client
                let len = self.rx_len.get();
                if let Some(buffer) = self.rx_buffer.take() {
                    // Prioritize HIL RX client (for MCTP) over native client
                    if self.hil_rx_client.is_some() {
                        self.hil_rx_client.map(|client| {
                            client.receive_write(buffer, len);
                        });
                    } else {
                        // Fall back to native client
                        self.client.map(|client| {
                            client.write_complete(buffer, len, Ok(()));
                        });
                    }
                }
                self.rx_len.set(0);
            }
            OperState::Read => {
                // Read complete - return TX buffer to client
                if let Some(buffer) = self.tx_buffer.take() {
                    // Prioritize HIL TX client (for MCTP) over native client
                    if self.hil_tx_client.is_some() {
                        self.hil_tx_client.map(|client| {
                            client.send_done(buffer, Ok(()));
                        });
                    }
                }
                self.tx_len.set(0);
                self.tx_idx.set(0);
            }
            OperState::Ibi => {
                // IBI complete (EVENT handler will handle hot-join separately)

                // If we have a TX buffer queued (from a pending read that wasn't completed),
                // we need to return it to the client
                if let Some(buffer) = self.tx_buffer.take() {
                    if self.hil_tx_client.is_some() {
                        self.hil_tx_client.map(|client| {
                            client.send_done(buffer, Err(ErrorCode::CANCEL));
                        });
                    }
                }
                self.tx_len.set(0);
                self.tx_idx.set(0);

                self.client.map(|client| client.ibi_complete(Ok(())));
            }
            OperState::Idle => {
                // Check for RXPEND in case there's unexpected data
                if (regs.status.get() & STATUS::RXPEND::SET.value) != 0 {
                    regs.datactrl.modify(DATACTRL::FLUSHFB::SET);
                }
            }
        }

        self.set_debug_gpio86(false);
    }

    /// Set RX buffer for receiving writes
    pub fn set_rx_buffer(&self, buffer: &'static mut [u8]) {
        self.rx_buffer.replace(buffer);
        self.rx_len.set(0);

        // RXPEND interrupt is already enabled in init(), so we're ready to receive data
    }

    /// Transmit data for read request
    pub fn transmit(
        &self,
        buffer: &'static mut [u8],
        len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        if self.state.get() != OperState::Idle && self.state.get() != OperState::Read {
            return Err((ErrorCode::BUSY, buffer));
        }

        if len > buffer.len() {
            return Err((ErrorCode::SIZE, buffer));
        }

        self.tx_buffer.replace(buffer);
        self.tx_len.set(len);
        self.tx_idx.set(0);

        // Flush TX FIFO to clear any residual data from previous transmissions
        // This prevents old data from being concatenated with new responses
        let regs = self.registers;
        regs.datactrl.modify(DATACTRL::FLUSHTB::SET);

        // If we're already in a read operation, start transmitting
        if self.state.get() == OperState::Read {
            self.handle_tx();
        } else {
            // Send IBI to notify controller that data is ready to be read
            // Only send MDB, the actual data will be read in the subsequent master read transaction
            let _ = self.send_ibi_with_len(MDB_PENDING_READ_MCTP, 0);

            // Set state to Read since we've prepared read response data
            // This ensures handle_start() will properly call send_done() when the next
            // command arrives via repeated START, returning tx_buffer to MCTP
            self.state.set(OperState::Read);

            // CRITICAL: Pre-fill TX FIFO immediately after IBI to prevent URUNNACK error
            // The master may respond to the IBI very quickly with a read request.
            // If the TX FIFO is empty when the master reads, it will cause URUNNACK (ERRWARN bit 2).
            if self.transfer_mode.get() == TransferMode::Fifo {
                self.tx_buffer.map(|buffer| {
                    let regs = self.registers;
                    let tx_len = self.tx_len.get();
                    let mut idx = 0;

                    // Fill TX FIFO with available data
                    while !regs.datactrl.is_set(DATACTRL::TXFULL) && idx < tx_len {
                        if idx == tx_len - 1 {
                            // Last byte - use WDATABE
                            regs.wdatabe
                                .set(WDATABE::DATA.val(buffer[idx] as u32).value);
                        } else {
                            // Not last byte - use WDATAB without END
                            regs.wdatab.set(WDATAB::DATA.val(buffer[idx] as u32).value);
                        }
                        idx += 1;
                    }

                    self.tx_idx.set(idx);
                });
            }

            // The controller should respond to the IBI by initiating a read transaction,
            // at which point handle_matched() will be called to continue the data transfer if needed.
        }

        Ok(())
    }

    /// Send In-Band Interrupt (IBI) with simple length parameter
    ///
    /// This is a simplified version similar to the runtime/kernel/drivers/i3c/src/core.rs implementation.
    /// It sends an IBI with MDB and a 2-byte length payload (big-endian).
    ///
    /// # Arguments
    /// * `mdb` - Mandatory Data Byte to send with the IBI
    /// * `len` - Length value to send as 2-byte payload (converted to big-endian)
    ///
    /// # Reference
    /// Based on runtime/kernel/drivers/i3c/src/core.rs send_ibi() implementation
    pub fn send_ibi_with_len(&self, mdb: u8, len: u16) -> Result<(), ErrorCode> {
        let regs = self.registers;

        // Check if IBI is disabled by the controller
        if regs.status.is_set(STATUS::IBIDIS) {
            return Err(ErrorCode::NOSUPPORT);
        }

        // Set state to IBI
        self.state.set(OperState::Ibi);

        // Write MDB (Mandatory Data Byte) to CTRL.IBIDATA field (bits 15:8)
        let mut ctrl_value = (mdb as u32) << 8;

        // Handle no extended data case
        if len == 0 {
            // No extended data
            // Trigger IBI by setting EVENT field to EmitIBI (0x1)
            ctrl_value |= 1;
            // Write the complete CTRL register value
            regs.ctrl.set(ctrl_value);
            return Ok(());
        }

        // Write 2-byte length payload (big-endian)
        let len_bytes = len.to_be_bytes();

        if self.transfer_mode.get() == TransferMode::Fifo {
            // Write first byte (MSB)
            regs.wdatab.set(WDATAB::DATA.val(len_bytes[0] as u32).value);
            // Write second byte (LSB) as end
            regs.wdatabe
                .set(WDATABE::DATA.val(len_bytes[1] as u32).value);
        }

        // Set IBIEXT1.CNT to 0
        regs.ibiext1.set(0);

        // Set EXTDATA bit (bit 3) to indicate extended data is present
        ctrl_value |= 1 << 3;

        // Trigger IBI by setting EVENT field to EmitIBI (0x1)
        ctrl_value |= 1;

        // Write the complete CTRL register value
        regs.ctrl.set(ctrl_value);

        Ok(())
    }

    /// Send In-Band Interrupt (IBI)
    ///
    /// # Arguments
    /// * `mdb` - Mandatory Data Byte to send with the IBI
    /// * `payload` - Optional extended IBI payload (up to 15 additional bytes)
    ///
    /// # Implementation based on Zephyr
    /// Reference: zephyr/drivers/i3c/i3c_npcm.c::npcm_i3c_target_ibi_raise()
    ///
    /// The IBI process:
    /// 1. Write MDB to CTRL.IBIDATA field (bits 15:8)
    /// 2. If extended payload exists:
    ///    - Write additional bytes to TX FIFO (WDATAB for all except last)
    ///    - Write last byte to WDATABE
    ///    - Set IBIEXT1.CNT to 0
    ///    - Set CTRL.EXTDATA bit
    /// 3. Trigger IBI by setting CTRL.EVENT to EmitIBI (0x1)
    pub fn send_ibi_with_payload(&self, mdb: u8, payload: Option<&[u8]>) -> Result<(), ErrorCode> {
        let regs = self.registers;

        // Check if IBI is disabled by the controller
        if regs.status.is_set(STATUS::IBIDIS) {
            return Err(ErrorCode::NOSUPPORT);
        }

        // Set state to IBI
        self.state.set(OperState::Ibi);

        // Step 1: Write MDB (Mandatory Data Byte) to CTRL.IBIDATA field (bits 15:8)
        // Clear any existing CTRL value and set the MDB in the IBIDATA field
        let mut ctrl_value = (mdb as u32) << 8; // IBIDATA is bits 15:8

        // Step 2: Handle extended payload if provided
        if let Some(ext_payload) = payload {
            if !ext_payload.is_empty() {
                // Write extended payload bytes to TX FIFO
                let len = ext_payload.len();

                // For FIFO mode, write bytes directly
                if self.transfer_mode.get() == TransferMode::Fifo {
                    for i in 0..len {
                        if i == len - 1 {
                            // Last byte - use WDATABE register
                            regs.wdatabe
                                .set(WDATABE::DATA.val(ext_payload[i] as u32).value);
                        } else {
                            // Not last byte - use WDATAB register
                            regs.wdatab
                                .set(WDATAB::DATA.val(ext_payload[i] as u32).value);
                        }
                    }
                }

                // Set IBIEXT1.CNT to 0 (as per Zephyr implementation)
                regs.ibiext1.set(0);

                // Set EXTDATA bit (bit 3) to indicate extended data is present
                ctrl_value |= 1 << 3; // EXTDATA bit
            }
        }

        // Step 3: Trigger IBI by setting EVENT field to EmitIBI (0x1)
        // EVENT field is bits 1:0
        ctrl_value |= 1; // EVENT = EmitIBI (0x1)

        // Write the complete CTRL register value
        regs.ctrl.set(ctrl_value);

        Ok(())
    }

    /// Request Hot-Join to join the I3C bus
    /// This allows a device without a static address to request dynamic address assignment
    pub fn request_hotjoin(&self) -> Result<(), ErrorCode> {
        let regs = self.registers;

        // Check if we're in a valid state to request hot-join
        if self.state.get() != OperState::Idle {
            return Err(ErrorCode::BUSY);
        }

        // Check if hot-join is disabled by the controller
        if regs.status.is_set(STATUS::HJDIS) {
            return Err(ErrorCode::NOSUPPORT);
        }

        // Set state to IBI (hot-join is a special type of IBI)
        self.state.set(OperState::Ibi);

        // Temporarily disable slave mode to emit hot-join
        regs.config.modify(CONFIG::SLVENA::Disabled);

        // Emit hot-join request
        regs.ctrl.set(CTRL::EVENT::EmitHotJoin.value);

        // Re-enable slave mode
        regs.config.modify(CONFIG::SLVENA::Enabled);

        Ok(())
    }

    /// Get current address (dynamic if available, else static)
    pub fn get_address(&self) -> u8 {
        self.dynamic_address
            .get()
            .unwrap_or(self.static_address.get())
    }

    /// Handle DMA interrupt
    pub fn handle_dma_interrupt(&self) {
        let dma = self.dma_registers;

        // Check RX channel completion
        let rx_mask = 1 << self.dma_rx_channel;
        if (dma.tdsts.get() & rx_mask) != 0 {
            // Clear interrupt flag
            dma.tdsts.set(rx_mask);

            // RX DMA complete - update length and notify client
            if self.state.get() == OperState::Write {
                self.rx_buffer.take().map(|buffer| {
                    // Get actual transferred length from DMA (if available in hardware)
                    let len = self.rx_len.get();
                    self.client.map(|client| {
                        client.write_complete(buffer, len, Ok(()));
                    });
                });
                self.rx_len.set(0);
                self.state.set(OperState::Idle);
            }
        }

        // Check TX channel completion
        let tx_mask = 1 << self.dma_tx_channel;
        if (dma.tdsts.get() & tx_mask) != 0 {
            // Clear interrupt flag
            dma.tdsts.set(tx_mask);

            // TX DMA complete - clean up buffer
            if self.state.get() == OperState::Read {
                self.tx_buffer.take();
                self.tx_len.set(0);
                self.tx_idx.set(0);
                self.state.set(OperState::Idle);
            }
        }

        // Check for DMA errors
        if dma.abtsts.get() != 0 {
            // Clear abort status
            dma.abtsts.set(0xFFFFFFFF);

            // Handle error - abort current transfer
            if self.state.get() == OperState::Write {
                self.rx_buffer.take().map(|buffer| {
                    self.client.map(|client| {
                        client.write_complete(buffer, 0, Err(ErrorCode::FAIL));
                    });
                });
            }
            self.state.set(OperState::Idle);
        }
    }

    /// Get transfer mode
    pub fn get_transfer_mode(&self) -> TransferMode {
        self.transfer_mode.get()
    }
}

// Implement the I3CTarget trait from i3c_driver for MCTP compatibility
impl<'a> hil::I3CTarget<'a> for I3cTarget<'a> {
    fn set_tx_client(&self, client: &'a dyn hil::TxClient) {
        self.hil_tx_client.set(client);
    }

    fn set_rx_client(&self, client: &'a dyn hil::RxClient) {
        self.hil_rx_client.set(client);
    }

    fn set_rx_buffer(&self, rx_buf: &'static mut [u8]) {
        self.set_rx_buffer(rx_buf);
    }

    fn transmit_read(
        &self,
        tx_buf: &'static mut [u8],
        len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        self.transmit(tx_buf, len)
    }

    fn enable(&self) {
        // Already enabled during init()
    }

    fn disable(&self) {
        // Could disable the peripheral if needed
    }

    fn get_device_info(&self) -> hil::I3CTargetInfo {
        hil::I3CTargetInfo {
            static_addr: Some(self.static_address.get()),
            dynamic_addr: self.dynamic_address.get(),
            max_read_len: self.max_read_len.get() as usize,
            max_write_len: self.max_write_len.get() as usize,
        }
    }
}
