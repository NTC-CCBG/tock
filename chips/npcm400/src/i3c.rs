// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2025.

//! I3C Target Driver for Nuvoton NPCM400
//!
//! This driver implements I3C target (slave) functionality for the NPCM400 chip.
//!
//! Hardware features:
//! - I3C target mode with dynamic address assignment
//! - FIFO-based RX/TX (16 bytes each)
//! - Interrupt-driven operation
//! - Support for private read/write transfers
//! - In-Band Interrupt (IBI) support

use crate::gpio;
use crate::i3c;
use crate::pdma::{
    Pdma, PdmaAddrMode, PdmaBurstSize, PdmaChannel, PdmaDirection, PdmaMode, PdmaPeripheral,
    PdmaScatterDescriptor, PdmaTransferConfig, PdmaWidth, PDMA,
};
use core::cell::Cell;
use kernel::utilities::cells::{OptionalCell, TakeCell};
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
/// Bit 0: IBI_REQUEST_CAPABLE = 1 (can generate IBIs)
/// Bit 1: IBI_PAYLOAD = 1 (IBI can have payload)
/// Bit 2: BCR[2] = 1
/// Bit 5: Offline capable = 1
const BUS_CHARACTERISTICS_TARGET: u8 = 0x27; // Target BCR with IBI capability
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

// Conditional debug macro for I3C verbose logging
#[cfg(feature = "debug-i3c-verbose")]
macro_rules! i3c_verbose {
    ($($arg:tt)*) => (kernel::debug!($($arg)*));
}

#[cfg(not(feature = "debug-i3c-verbose"))]
macro_rules! i3c_verbose {
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

/// DMA channel assignments for I3C buses
/// Each I3C bus uses 2 DMA channels (RX and TX)
const I3C1_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel0;
const I3C1_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel1;
const I3C2_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel2;
const I3C2_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel3;
const I3C3_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel4;
const I3C3_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel5;
const I3C4_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel6;
const I3C4_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel7;
const I3C5_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel8;
const I3C5_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel9;
const I3C6_DMA_RX_CHANNEL: PdmaChannel = PdmaChannel::Channel10;
const I3C6_DMA_TX_CHANNEL: PdmaChannel = PdmaChannel::Channel11;

const I3C1_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c1Rx;
const I3C1_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c1Tx;
const I3C2_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c2Rx;
const I3C2_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c2Tx;
const I3C3_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c3Rx;
const I3C3_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c3Tx;
const I3C4_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c4Rx;
const I3C4_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c4Tx;
const I3C5_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c5Rx;
const I3C5_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c5Tx;
const I3C6_DMA_RX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c6Rx;
const I3C6_DMA_TX_PERIPHERAL: PdmaPeripheral = PdmaPeripheral::I3c6Tx;

const PDMA_MAX_TRANSFER_COUNT: usize = 0x3FFF;

register_structs! {
    /// NPCM I3C Target Register Map
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
        /// 0x020: Target DMA Control
        (0x020 => dmactrl: ReadWrite<u32, DMACTRL::Register>),
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
        /// 0x054: Target Byte-Only Write Byte Data (used for DMA)
        (0x054 => wdatab1: ReadWrite<u8>),
        /// 0x055-0x05F: Reserved
        (0x055 => _reserved4: [u8; 0x0B]),
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
        /// Write Overrun
        OWRITE OFFSET(17) NUMBITS(1) [],
        /// Read Underrun
        OREAD OFFSET(16) NUMBITS(1) [],
        /// S0 or S1 error
        S0S1 OFFSET(11) NUMBITS(1) [],
        /// HDR-DDR CRC Error
        HCRC OFFSET(10) NUMBITS(1) [],
        /// HDR parity error
        HPAR OFFSET(9) NUMBITS(1) [],
        /// SDR parity Error
        SPAR OFFSET(8) NUMBITS(1) [],
        /// Invalid start
        INVSTART OFFSET(4) NUMBITS(1) [],
        /// Termination error
        TERM OFFSET(3) NUMBITS(1) [],
        /// Underrun NACK
        URUNNACK OFFSET(2) NUMBITS(1) [],
        /// Underflow
        URUN OFFSET(1) NUMBITS(1) [],
        /// Overflow
        ORUN OFFSET(0) NUMBITS(1) []
    ],

    /// DNA Control Register (DMACTRL)
    DMACTRL [
        /// DMA data width
        DMAWIDTH OFFSET(4) NUMBITS(2) [
            Byte = 1,
            Word = 2 // 16 bits
        ],
        /// DMA write to bus enable
        DMATB OFFSET(2) NUMBITS(2) [
            Disabled = 0,
            Enabled = 2
        ],
        /// DMA from bus enable
        DMAFB OFFSET(0) NUMBITS(2) [
            Disabled = 0,
            Enabled = 2
        ]
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
    pdma: &'static Pdma,

    /// Current operation state
    state: Cell<OperState>,

    /// IBI pending flag - set when IBI is sent, cleared when EVENT completes
    ibi_pending: Cell<bool>,

    /// Transfer mode (FIFO or DMA)
    transfer_mode: Cell<TransferMode>,

    /// RX buffer for incoming writes
    rx_buffer: TakeCell<'static, [u8]>,
    rx_len: Cell<usize>,

    /// TX buffer for outgoing reads
    tx_buffer: TakeCell<'static, [u8]>,
    tx_len: Cell<usize>,
    tx_idx: Cell<usize>,

    /// DMA channels and routing
    dma_rx_channel: PdmaChannel,
    dma_tx_channel: PdmaChannel,
    dma_rx_peripheral: PdmaPeripheral,
    dma_tx_peripheral: PdmaPeripheral,
    rx_dma_expected: Cell<u16>,
    rx_dma_active: Cell<bool>,
    tx_dma_expected: Cell<u16>,
    tx_dma_active: Cell<bool>,

    /// Scatter-gather descriptor for RX DMA (must be 4-byte aligned)
    rx_sg_descriptor: PdmaScatterDescriptor,
    /// Scatter-gather descriptors for TX DMA (must be 4-byte aligned)
    /// Descriptor 0 transfers N-1 bytes to WDATAB
    /// Descriptor 1 transfers last byte to WDATABE (with END bit)
    tx_sg_descriptor: [PdmaScatterDescriptor; 2],

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
    pub fn new_i3c1() -> Self {
        Self::new_with_params(
            I3C1_BASE,
            I3C1_DMA_RX_CHANNEL,
            I3C1_DMA_TX_CHANNEL,
            I3C1_DMA_RX_PERIPHERAL,
            I3C1_DMA_TX_PERIPHERAL,
        )
    }

    /// Create I3C2 instance
    pub fn new_i3c2() -> Self {
        Self::new_with_params(
            I3C2_BASE,
            I3C2_DMA_RX_CHANNEL,
            I3C2_DMA_TX_CHANNEL,
            I3C2_DMA_RX_PERIPHERAL,
            I3C2_DMA_TX_PERIPHERAL,
        )
    }

    /// Create I3C3 instance
    pub fn new_i3c3() -> Self {
        Self::new_with_params(
            I3C3_BASE,
            I3C3_DMA_RX_CHANNEL,
            I3C3_DMA_TX_CHANNEL,
            I3C3_DMA_RX_PERIPHERAL,
            I3C3_DMA_TX_PERIPHERAL,
        )
    }

    /// Create I3C4 instance
    pub fn new_i3c4() -> Self {
        Self::new_with_params(
            I3C4_BASE,
            I3C4_DMA_RX_CHANNEL,
            I3C4_DMA_TX_CHANNEL,
            I3C4_DMA_RX_PERIPHERAL,
            I3C4_DMA_TX_PERIPHERAL,
        )
    }

    /// Create I3C5 instance
    pub fn new_i3c5() -> Self {
        Self::new_with_params(
            I3C5_BASE,
            I3C5_DMA_RX_CHANNEL,
            I3C5_DMA_TX_CHANNEL,
            I3C5_DMA_RX_PERIPHERAL,
            I3C5_DMA_TX_PERIPHERAL,
        )
    }

    /// Create I3C6 instance
    pub fn new_i3c6() -> Self {
        Self::new_with_params(
            I3C6_BASE,
            I3C6_DMA_RX_CHANNEL,
            I3C6_DMA_TX_CHANNEL,
            I3C6_DMA_RX_PERIPHERAL,
            I3C6_DMA_TX_PERIPHERAL,
        )
    }

    /// Internal constructor with parameters
    fn new_with_params(
        registers: StaticRef<I3cRegisters>,
        dma_rx_channel: PdmaChannel,
        dma_tx_channel: PdmaChannel,
        dma_rx_peripheral: PdmaPeripheral,
        dma_tx_peripheral: PdmaPeripheral,
    ) -> Self {
        // Default PID = 0x0632_12344567 (48 bits)
        // Vendor=0x0319, Part=0x1234, Instance=0x4, Extra=0x567
        let default_pid = ProvisionedId::new(0x0319, 0x1234, 0x4, 0x567);

        // Verify RX and TX use different channels
        if dma_rx_channel as u8 == dma_tx_channel as u8 {
            panic!("I3C: RX and TX DMA channels must be different!");
        }

        Self {
            registers,
            pdma: &PDMA,
            state: Cell::new(OperState::Idle),
            ibi_pending: Cell::new(false),
            transfer_mode: Cell::new(TransferMode::Dma),
            rx_buffer: TakeCell::empty(),
            rx_len: Cell::new(0),
            tx_buffer: TakeCell::empty(),
            tx_len: Cell::new(0),
            tx_idx: Cell::new(0),
            dma_rx_channel,
            dma_tx_channel,
            dma_rx_peripheral,
            dma_tx_peripheral,
            rx_dma_expected: Cell::new(0),
            rx_dma_active: Cell::new(false),
            tx_dma_expected: Cell::new(0),
            tx_dma_active: Cell::new(false),
            rx_sg_descriptor: PdmaScatterDescriptor::new(),
            tx_sg_descriptor: [PdmaScatterDescriptor::new(), PdmaScatterDescriptor::new()],
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
    pub fn new() -> Self {
        Self::new_i3c1()
    }

    /// Enable DMA mode for high-speed transfers
    pub fn enable_rx_dma(&self) {
        self.registers.dmactrl.modify(DMACTRL::DMAFB::Enabled);
    }

    /// Disable DMA mode
    pub fn disable_rx_dma(&self) {
        self.registers.dmactrl.modify(DMACTRL::DMAFB::Disabled);
    }

    /// Enable DMA writes from memory to the TX FIFO
    pub fn enable_tx_dma(&self) {
        i3c_verbose!("[S]");

        self.registers.dmactrl.modify(DMACTRL::DMATB::Enabled);
    }

    /// Disable DMA writes to TX FIFO so software can refill FIFO directly
    pub fn disable_tx_dma(&self) {
        i3c_verbose!("[P]");

        self.registers.dmactrl.modify(DMACTRL::DMATB::Disabled);
    }

    /// Start a PDMA RX transfer unconditionally
    fn start_rx_dma_transfer(&self) -> bool {
        // Configure RX DMA with the provided buffer
        self.rx_buffer.map_or(false, |buffer| {
            if self.configure_rx_dma(buffer).is_ok() {
                self.enable_rx_dma();
                true
            } else {
                false
            }
        })
    }

    /// Start a PDMA RX transfer when DMA mode is active and resources are ready.
    fn start_rx_dma_if_needed(&self) -> bool {
        if self.transfer_mode.get() != TransferMode::Dma {
            return false;
        }

        if self.rx_dma_active.get() {
            return false;
        }

        self.start_rx_dma_transfer()
    }

    /// Configure and launch a PDMA transfer from the RX FIFO into the provided buffer.
    /// Uses scatter-gather mode.
    ///
    /// The configuration follows this sequence (matching npcm_i3c_pdma_configure):
    /// 1. Initialize top descriptor table (hardware DSCT) with ScatterGather mode
    /// 2. Set SCATBA register with upper 16 bits of SG descriptor address
    /// 3. Configure scatter-gather descriptor (in RAM) with actual transfer parameters in Basic mode
    /// 4. Clear any previous transfer done flags
    /// 5. Configure peripheral request source
    /// 6. Enable interrupts for the channel
    /// 7. Enable the PDMA channel
    /// 8. Enable I3C DMA
    fn configure_rx_dma(&self, buffer: &mut [u8]) -> Result<(), ErrorCode> {
        if buffer.is_empty() {
            return Err(ErrorCode::SIZE);
        }

        // Stop any previous RX DMA
        self.disable_rx_dma();

        let limited_len = buffer.len().min(PDMA_MAX_TRANSFER_COUNT);
        let transfer_count = limited_len as u16;

        let sg_desc_addr = &self.rx_sg_descriptor as *const _ as u32;

        // Step 1: Clear any previous transfer done flags
        self.pdma.clear_transfer_done(self.dma_rx_channel);

        // Step 2: Configure peripheral request selection (setup channel request selection)
        self.pdma
            .configure_peripheral_request(self.dma_rx_channel, self.dma_rx_peripheral);

        // Step 3: Initialize top descriptor table (hardware DSCT register) with ScatterGather mode
        self.pdma
            .init_scatter_gather_descriptor(self.dma_rx_channel, sg_desc_addr);

        // Step 4: Configure scatter-gather table base MSB address
        self.pdma
            .set_scatter_gather_base(sg_desc_addr & 0xFFFF_0000);

        // Step 5: Build ctrl value and configure the scatter-gather descriptor (in RAM)
        let sg_config = PdmaTransferConfig {
            source_addr: &self.registers.rdatab as *const _ as u32,
            dest_addr: buffer.as_mut_ptr() as u32,
            transfer_count: transfer_count.saturating_sub(1), // TXCNT is (count - 1) per hardware spec
            src_width: PdmaWidth::Byte,
            dst_width: PdmaWidth::Byte,
            src_addr_mode: PdmaAddrMode::Fixed, // Fixed source (I3C RX FIFO)
            dst_addr_mode: PdmaAddrMode::Increment, // Increment destination (buffer)
            direction: PdmaDirection::PeriphToMem,
            peripheral: Some(self.dma_rx_peripheral),
            burst_size: PdmaBurstSize::Burst1, // Single transfer mode
            mode: PdmaMode::Basic,             // Last descriptor uses Basic mode
            next: None,                        // No next descriptor (last in chain)
        };

        self.rx_sg_descriptor.configure(&sg_config)?;

        // Step 6: Enable interrupts for this channel (both TD and SGTD)
        self.pdma.enable_interrupts(self.dma_rx_channel);

        self.rx_dma_expected.set(transfer_count);
        self.rx_dma_active.set(true);
        self.rx_len.set(0);

        // Step 7: Enable the channel (start PDMA)
        // i3c_debug!(
        //     "[I3C DMA] RX channel={} cfg:dest=0x{:08x} len={}",
        //     self.dma_rx_channel as u8,
        //     sg_config.dest_addr,
        //     limited_len
        // );
        self.pdma.enable_channel(self.dma_rx_channel);

        // Debug: Read back hardware descriptor values
        // self.pdma.debug_descriptor(self.dma_rx_channel);

        Ok(())
    }

    /// Finalise an active RX DMA transfer and return the number of bytes received.
    /// Returns the ACTUAL number of bytes received (not the configured size).
    ///
    /// In scatter-gather mode, the hardware updates the SG descriptor in RAM,
    /// not the hardware DSCT register, so we read from the SG descriptor.
    fn finalize_rx_dma(&self) -> usize {
        if self.transfer_mode.get() != TransferMode::Dma || !self.rx_dma_active.get() {
            return self.rx_len.get();
        }

        // Get remaining transfer count from PDMA hardware
        let remaining = self.pdma.remaining_transfers(self.dma_rx_channel);
        let expected = self.rx_dma_expected.get();

        // Calculate actual bytes received: expected - remaining
        let actual_received = expected.saturating_sub(remaining) as usize;

        // Stop the DMA channel
        self.pdma.disable_channel(self.dma_rx_channel);
        self.rx_dma_active.set(false);
        self.rx_dma_expected.set(0);

        // Store actual received length
        self.rx_len.set(actual_received);

        // Disable I3C DMA handshaking
        self.disable_rx_dma();

        actual_received
    }

    /// Start a PDMA TX transfer if resources are ready.
    fn start_tx_dma_if_needed(&self) -> bool {
        if self.transfer_mode.get() != TransferMode::Dma {
            i3c_verbose!("[TX DMA] !DMA");
            return false;
        }

        if self.tx_dma_active.get() {
            i3c_verbose!("[TX DMA] active");
            return false;
        }

        if self.tx_buffer.is_none() {
            i3c_verbose!("[TX DMA] !buf");
            return false;
        }

        self.start_tx_dma_transfer()
    }

    /// Start a PDMA TX transfer unconditionally.
    /// Returns true if the transfer was successfully configured and started, false otherwise.
    fn start_tx_dma_transfer(&self) -> bool {
        self.tx_buffer.map_or(false, |buffer| {
            let len = self.tx_len.get().min(buffer.len());
            if len == 0 {
                return false;
            }

            if self.configure_tx_dma(&buffer[..len]).is_ok() {
                self.enable_tx_dma();
                true
            } else {
                false
            }
        })
    }

    /// Configure TX scatter-gather descriptor and kick off DMA towards the TX FIFO.
    fn configure_tx_dma(&self, buffer: &[u8]) -> Result<(), ErrorCode> {
        if buffer.is_empty() {
            return Err(ErrorCode::SIZE);
        }

        i3c_verbose!("=c");

        // Stop any previous TX DMA
        self.disable_tx_dma();

        // Clear any previous completion
        self.pdma.clear_transfer_done(self.dma_tx_channel);

        // Route PDMA requests to the TX peripheral handshake
        self.pdma
            .configure_peripheral_request(self.dma_tx_channel, self.dma_tx_peripheral);

        // Configure scatter-gather base
        let sg_desc_addr = &self.tx_sg_descriptor[0] as *const _ as u32;

        self.pdma
            .init_scatter_gather_descriptor(self.dma_tx_channel, sg_desc_addr);
        let sg_base = sg_desc_addr & 0xFFFF_0000;
        self.pdma.set_scatter_gather_base(sg_base);

        // Use two-descriptor chain
        if buffer.len() == 1 {
            // Single byte - write directly to WDATABE
            let sg_config = PdmaTransferConfig {
                source_addr: buffer.as_ptr() as u32,
                dest_addr: &self.registers.wdatabe as *const _ as u32,
                transfer_count: 0, // 0 means 1 byte
                src_width: PdmaWidth::Byte,
                dst_width: PdmaWidth::Byte,
                src_addr_mode: PdmaAddrMode::Fixed,
                dst_addr_mode: PdmaAddrMode::Fixed,
                direction: PdmaDirection::MemToPeriph,
                peripheral: Some(self.dma_tx_peripheral),
                burst_size: PdmaBurstSize::Burst1,
                mode: PdmaMode::Basic,
                next: None,
            };
            self.tx_sg_descriptor[0].configure(&sg_config)?;
            self.tx_dma_expected.set(0);
        } else {
            // Multi-byte: use scatter-gather with two descriptors
            // Descriptor 0: Transfer (len - 2) bytes to WDATAB
            let first_count = (buffer.len() - 2) as u16;

            let sg_config_0 = PdmaTransferConfig {
                source_addr: buffer.as_ptr() as u32,
                dest_addr: &self.registers.wdatab1 as *const _ as u32,
                transfer_count: first_count, // Hardware adds 1, so this transfers first_count+1 bytes
                src_width: PdmaWidth::Byte,
                dst_width: PdmaWidth::Byte,
                src_addr_mode: PdmaAddrMode::Increment,
                dst_addr_mode: PdmaAddrMode::Fixed,
                direction: PdmaDirection::MemToPeriph,
                peripheral: Some(self.dma_tx_peripheral),
                burst_size: PdmaBurstSize::Burst1,
                mode: PdmaMode::ScatterGather,
                next: Some(&self.tx_sg_descriptor[1] as *const _ as u32),
            };

            self.tx_sg_descriptor[0].configure(&sg_config_0)?;

            // Descriptor 1: Transfer last byte to WDATABE (with END bit)
            let last_byte_addr = unsafe { buffer.as_ptr().add(buffer.len() - 1) as u32 };
            let sg_config_1 = PdmaTransferConfig {
                source_addr: last_byte_addr,
                dest_addr: &self.registers.wdatabe as *const _ as u32,
                transfer_count: 0, // 0 means 1 byte
                src_width: PdmaWidth::Byte,
                dst_width: PdmaWidth::Byte,
                src_addr_mode: PdmaAddrMode::Fixed,
                dst_addr_mode: PdmaAddrMode::Fixed,
                direction: PdmaDirection::MemToPeriph,
                peripheral: Some(self.dma_tx_peripheral),
                burst_size: PdmaBurstSize::Burst1,
                mode: PdmaMode::Basic,
                next: None,
            };

            self.tx_sg_descriptor[1].configure(&sg_config_1)?;

            // Expected is total bytes (for two-descriptor scatter-gather)
            // D0 transfers (len-1) bytes, D1 transfers 1 byte, total = len bytes
            self.tx_dma_expected.set(buffer.len() as u16);
        }

        self.pdma.enable_interrupts(self.dma_tx_channel);

        self.tx_dma_active.set(true);
        self.tx_idx.set(0);

        self.pdma.enable_channel(self.dma_tx_channel);

        Ok(())
    }

    /// Stop an in-flight TX DMA transfer and report bytes pushed into the FIFO.
    fn finalize_tx_dma(&self) -> usize {
        if self.transfer_mode.get() != TransferMode::Dma || !self.tx_dma_active.get() {
            i3c_verbose!("[TX DMA] not active");
            return self.tx_idx.get();
        }

        let remaining = self.pdma.remaining_transfers(self.dma_tx_channel);
        let expected = self.tx_dma_expected.get();
        let actual = expected.saturating_sub(remaining) as usize;

        self.pdma.disable_channel(self.dma_tx_channel);
        self.tx_dma_active.set(false);
        self.tx_dma_expected.set(0);

        self.tx_idx.set(actual);

        i3c_verbose!("=e");

        self.disable_tx_dma();

        actual
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
            i3c_debug!("[I3C Target driver] I3C reset_module: Unknown I3C instance");
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

        // Flush FIFOs - use original behavior (enable DMA for initialization)
        self.flush_rx_fifo(true);
        self.flush_tx_fifo(true);

        // Kick off RX DMA if resources are present
        if self.transfer_mode.get() == TransferMode::Dma {
            self.rx_dma_active.set(true);
        }

        gpio::enable_debug_gpio86_87_95_94();
    }

    /// Handle I3C interrupt
    /// This function reads the interrupt status, identifies the source of the interrupt,
    /// and calls the appropriate handler for each interrupt type.
    /// When the interrupt is not handled, the tock kernel may hang or misbehave.
    ///
    /// Loop until all interrupts are cleared to handle cases where new interrupts arrive during
    /// processing.
    pub fn handle_interrupt(&self) {
        let regs = self.registers;
        let mut int_masked = self.registers.intmasked.get();

        // Return early if no interrupts (spurious interrupt)
        if int_masked == 0 {
            // Re-read INTMASKED in case of race condition
            int_masked = self.registers.intmasked.get();
            if int_masked == 0 {
                i3c_verbose!("[I3C Target driver] Spurious Interrupt");
                return;
            }
        }

        gpio::set_debug_gpio87(true);

        // Loop until all interrupts are processed
        // This ensures new interrupts that arrive during processing are also handled
        while int_masked != 0 {
            // Handle START condition (includes repeated start)
            if (int_masked & INTMASKED::START::SET.value) != 0 {
                self.handle_start();

                // Clear START interrupt (W1C)
                regs.status.set(STATUS::START::SET.value);
            }

            // Handle SLVSTART (target reset detection)
            if (int_masked & INTMASKED::SLVSTART::SET.value) != 0 {
                i3c_debug!("[I3C Target driver] SLVSTART detected");

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
                i3c_verbose!("[I3C Target driver] RXPEND interrupt");

                self.handle_rxpend();
                // RXPEND auto-clears when FIFO is empty
            }

            // Handle STOP condition
            // Process AFTER RXPEND to ensure all data is read before completing transfer
            if (int_masked & INTMASKED::STOP::SET.value) != 0 {
                // i3c_verbose!("[I3C Target driver] STOP condition");

                self.handle_stop();

                // Clear STOP interrupt
                regs.status.set(STATUS::STOP::SET.value);
            }

            // Handle dynamic address change
            if (int_masked & INTMASKED::DACHG::SET.value) != 0 {
                i3c_verbose!("[I3C Target driver] Dynamic Address Change");

                self.handle_dachg();

                // Clear DACHG interrupt (W1C)
                regs.status.set(STATUS::DACHG::SET.value);
            }

            // Handle TX not full (read from controller)
            if (int_masked & INTMASKED::TXNOTFULL::SET.value) != 0 {
                i3c_debug!("[I3C Target driver] TX not full");

                // Clear TXNOTFULL interrupt (W1C)
                regs.status.set(STATUS::TXNOTFULL::SET.value);
            }

            // Handle CCC (Common Command Code)
            if (int_masked & INTMASKED::CCC::SET.value) != 0 {
                i3c_debug!("[I3C Target driver] CCC received");

                // Clear CCC interrupt (W1C)
                regs.status.set(STATUS::CCC::SET.value);
            }

            // Handle DDRMATCH
            if (int_masked & INTMASKED::DDRMATCH::SET.value) != 0 {
                i3c_debug!("[I3C Target driver] DDRMATCH event");

                // Clear DDRMATCH interrupt (W1C)
                regs.status.set(STATUS::DDRMATCH::SET.value);
            }

            // Handle CHANDLED
            if (int_masked & INTMASKED::CHANDLED::SET.value) != 0 {
                i3c_debug!("[I3C Target driver] CHANDLED event");

                // Flush FIFOs, the cmd code will remain in the buffer
                self.flush_rx_fifo(true);
                self.flush_tx_fifo(true);

                // Clear CHANDLED interrupt (W1C)
                regs.status.set(STATUS::CHANDLED::SET.value);
            }

            // Handle errors
            if (int_masked & INTMASKED::ERRWARN::SET.value) != 0 {
                self.handle_error();
            }

            // Re-check interrupts to handle any new interrupts that arrived during processing
            int_masked = regs.intmasked.get();
        }

        gpio::set_debug_gpio87(false);
    }

    /// Handle error condition
    fn handle_error(&self) {
        let regs = self.registers;
        let err = regs.errwarn.get();

        // Show errors
        if err != 0 {
            if regs.errwarn.is_set(ERRWARN::OWRITE) {
                i3c_debug!("[I3C Target driver] ERRWARN: Write overrun");
            }
            if regs.errwarn.is_set(ERRWARN::OREAD) {
                i3c_debug!("[I3C Target driver] ERRWARN: Read underrun");
            }
            if regs.errwarn.is_set(ERRWARN::S0S1) {
                i3c_debug!("[I3C Target driver] ERRWARN: S0 or S1 error");
            }
            if regs.errwarn.is_set(ERRWARN::HCRC) {
                i3c_debug!("[I3C Target driver] ERRWARN: HDR-DDR CRC error");
            }
            if regs.errwarn.is_set(ERRWARN::HPAR) {
                i3c_debug!("[I3C Target driver] ERRWARN: HDR parity error");
            }
            if regs.errwarn.is_set(ERRWARN::SPAR) {
                i3c_debug!("[I3C Target driver] ERRWARN: SDR parity error");
            }
            if regs.errwarn.is_set(ERRWARN::INVSTART) {
                i3c_debug!("[I3C Target driver] ERRWARN: Invalid start");
            }
            if regs.errwarn.is_set(ERRWARN::TERM) {
                i3c_debug!("[I3C Target driver] ERRWARN: Termination error");
            }
            if regs.errwarn.is_set(ERRWARN::URUNNACK) {
                i3c_debug!("[I3C Target driver] ERRWARN: Underrun NACK");
            }
            if regs.errwarn.is_set(ERRWARN::URUN) {
                i3c_debug!("[I3C Target driver] ERRWARN: Underflow");
            }
            if regs.errwarn.is_set(ERRWARN::ORUN) {
                i3c_debug!("[I3C Target driver] ERRWARN: Overflow");
            }
        }

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
                    let use_fifo =
                        self.transfer_mode.get() != TransferMode::Dma || !self.rx_dma_active.get();
                    if use_fifo {
                        let mut len = self.rx_len.get();
                        while regs.status.is_set(STATUS::RXPEND) && len < buffer.len() {
                            buffer[len] = regs.rdatab.read(RDATAB::DATA) as u8;
                            len += 1;
                        }
                        self.rx_len.set(len);
                    }
                });
            } else if self.transfer_mode.get() == TransferMode::Fifo
                || (self.transfer_mode.get() == TransferMode::Dma && !self.rx_dma_active.get())
            {
                self.flush_rx_fifo(true);
            }
        } else if self.transfer_mode.get() == TransferMode::Fifo
            || (self.transfer_mode.get() == TransferMode::Dma && !self.rx_dma_active.get())
        {
            self.flush_rx_fifo(true);
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
                    self.finalize_rx_dma();
                    self.rx_buffer.take().map(|buffer| {
                        self.client.map(|client| {
                            client.write_complete(buffer, 0, Err(ErrorCode::CANCEL));
                        });
                    });
                    self.rx_len.set(0);
                }
                OperState::Read => {
                    i3c_verbose!("-SL");
                    i3c_debug!("[I3C Target driver] Abort read due to SLVSTART");

                    // Abort read transfer
                    let _ = self.finalize_tx_dma();
                    self.tx_buffer.take();
                    self.tx_len.set(0);
                    self.tx_idx.set(0);
                }
                OperState::Ibi => {
                    i3c_debug!("[I3C Target driver] Abort IBI due to SLVSTART");

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
                self.finalize_rx_dma();
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
                i3c_verbose!("-S");

                // Repeated start during read - complete the read transfer
                let _ = self.finalize_tx_dma();
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

        gpio::set_debug_gpio95(true);

        // Timing delay to allow hardware to settle before reading STATUS register
        // At 96 MHz CPU clock: ~4 cycles per iteration (including loop overhead)
        // FIFO mode: ~1µs delay for byte transfer time at 12.5MHz I3C clock
        // DMA mode: ~0.25µs delay (DMA handshaking is faster)
        const CPU_MHZ: u32 = 96;
        let delay_cycles = if self.transfer_mode.get() == TransferMode::Fifo {
            CPU_MHZ // ~1µs at 96MHz (96 cycles ÷ 4 cycles/iter = 24 iterations)
        } else {
            CPU_MHZ / 4 // ~0.25µs for DMA mode
        };

        for _ in 0..(delay_cycles / 4) {
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
                i3c_debug!("[I3C Target driver] MATCHED: Cancelling pending TX");
                let _ = self.finalize_tx_dma();
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

            // Start RX DMA if in DMA mode and not already active
            if self.transfer_mode.get() == TransferMode::Dma {
                self.start_rx_dma_if_needed();
            }

            // Flush TX FIFO to clear any stale response data from previous read
            // This prevents old data from being sent if master reads without us preparing a response
            self.flush_tx_fifo(true);
        } else {
            // Read request from controller - target must send data
            // If transmit() sent IBI, it already set state to Read and prefilled FIFO
            // Otherwise, just transition to Read state
            self.state.set(OperState::Read);

            // If no buffer is available, request data from client
            if self.tx_buffer.is_none() {
                // i3c_verbose!("[I3C Target driver] MATCHED: No TX buffer available");
                // self.client.map(|client| client.read_requested());

                // No need to check if we use TX DMA, the buffer is already provided in transmit()
            }

            // In FIFO mode, we must immediately fill TX FIFO before handle_tx is called
            // because the controller will start clocking data right after address match.
            // This is critical for timing - it may be too late if we wait for handle_tx.
            let should_fill_fifo = self.transfer_mode.get() == TransferMode::Fifo
                || (self.transfer_mode.get() == TransferMode::Dma && !self.tx_dma_active.get());
            if should_fill_fifo {
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

            // For DMA mode, TX DMA already enabled in EVENT handler if needed
        }

        gpio::set_debug_gpio95(false);
    }

    /// Handle EVENT interrupt (IBI, hot-join, controller role request)
    /// For simplicity, we only handle IBI and hot-join here.
    fn handle_event(&self) {
        gpio::set_debug_gpio94(true);

        let regs = self.registers;
        let evdet = (regs.status.get() >> 20) & 0x3; // EVDET field

        // EVDET values:
        // 0 = No event or event not yet sent
        // 1 = Event request unable to be sent, wait for bus idle
        // 2 = Event request sent and NACKed
        // 3 = Event request sent and ACKed
        match evdet {
            3 => {
                // Event acknowledged
                // Check if this was an IBI for pending read data (from transmit())
                if self.ibi_pending.get() {
                    i3c_verbose!("E");

                    // Clear the pending flag
                    self.ibi_pending.set(false);

                    // Transition to Read state
                    // self.state.set(OperState::Read);

                    // CRITICAL: Pre-fill TX FIFO to prevent URUNNACK error
                    // The controller may respond to the IBI very quickly with a read request.
                    // If the TX FIFO is empty when the controller reads, it will cause URUNNACK
                    if self.transfer_mode.get() == TransferMode::Fifo {
                        self.tx_buffer.map(|buffer| {
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
                    } else if self.transfer_mode.get() == TransferMode::Dma {
                        // TX DMA already started in transmit(), just ensure it's active
                        self.start_tx_dma_if_needed();
                    }
                } else {
                    // // This was a hot-join request
                    // self.client.map(|client| {
                    //     client.hotjoin_complete(Ok(()));
                    // });
                    // self.state.set(OperState::Idle);

                    i3c_debug!("[I3C Target driver] Hot-join event acknowledged");
                }
            }
            1 | 2 => {
                // Event rejected or unable to send
                if self.ibi_pending.get() {
                    // Clear the pending flag and return to Idle
                    self.ibi_pending.set(false);
                    self.state.set(OperState::Idle);

                    // Return the TX buffer since we won't be transmitting
                    self.tx_buffer.take().map(|buffer| {
                        self.hil_tx_client.map(|client| {
                            client.send_done(buffer, Err(kernel::ErrorCode::FAIL));
                        });
                    });
                } else {
                    // Hot-join failed
                    self.client.map(|client| {
                        client.hotjoin_complete(Err(ErrorCode::FAIL));
                    });
                    self.state.set(OperState::Idle);

                    i3c_debug!("[I3C Target driver] Hot-join Set idle");
                }
            }
            _ => {
                i3c_debug!("[I3C Target driver] EVENT: No event or not yet sent");

                // No event or not yet sent - do nothing
            }
        }

        gpio::set_debug_gpio94(false);
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

        if self.transfer_mode.get() == TransferMode::Dma {
            if self.start_tx_dma_if_needed() {
                return;
            }
        }

        // Check transfer mode
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

    /// Handle STOP condition
    ///
    /// STOP condition indicates the end of a transfer on the I3C bus.
    /// According to the hardware behavior, START and STOP can occur simultaneously
    /// (race condition), so we need to check for and clear any concurrent START interrupt.
    fn handle_stop(&self) {
        let regs = self.registers;
        let current_state = self.state.get();
        let int_masked = regs.intmasked.get();

        gpio::set_debug_gpio86(true);

        // Check for concurrent START interrupt
        // If START is also pending, clear it to avoid processing it separately
        // This can happen in certain timing conditions on the bus
        if (int_masked & INTMASKED::START::SET.value) != 0 {
            // Clear the START status bit (W1C)
            regs.status.set(STATUS::START::SET.value);
        }

        // Check for EVENT interrupt (IBI, hot-join, controller role request) FIRST
        // This speeds up TX FIFO preparation for IBI-triggered reads
        if (int_masked & INTMASKED::EVENT::SET.value) != 0 {
            i3c_verbose!("EV");
            self.handle_event();

            // Clear EVENT interrupt (W1C)
            regs.status.set(STATUS::EVENT::SET.value);
        }

        // Process the transfer completion based on previous state
        match current_state {
            OperState::Write => {
                i3c_verbose!("W");

                // Write complete - finalize DMA and get actual received length
                let actual_len = self.finalize_rx_dma();

                // Return to idle state BEFORE calling client callbacks
                // This allows clients (e.g., MCTP) to immediately queue TX data in response
                // The client may call transmit() which will transition to Read state
                // Only set to Idle if handle_event() didn't already change the state
                if self.state.get() == current_state {
                    self.state.set(OperState::Idle);
                }

                if let Some(buffer) = self.rx_buffer.take() {
                    // Prioritize HIL RX client (for MCTP) over native client
                    if self.hil_rx_client.is_some() {
                        self.hil_rx_client.map(|client| {
                            client.receive_write(buffer, actual_len);
                        });
                    } else {
                        // Fall back to native client
                        self.client.map(|client| {
                            client.write_complete(buffer, actual_len, Ok(()));
                        });
                    }
                } else {
                    i3c_debug!("[I3C] Write STOP: NO RX buffer!");
                }

                // NOTE: DMA will be re-enabled when the client provides a new buffer via set_rx_buffer()
                // This ensures we're ready for the next transfer
            }
            OperState::Read => {
                i3c_verbose!("R");

                // gpio::set_debug_gpio95(true);

                // Return to idle state
                // Only set to Idle if handle_event() didn't already change the state
                if self.state.get() == current_state {
                    self.state.set(OperState::Idle);
                }

                // Read complete - return TX buffer to client
                let _ = self.finalize_tx_dma();
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

                // gpio::set_debug_gpio95(false);
            }
            OperState::Ibi => {
                i3c_verbose!("B");

                // handle_event() will check EVDET status and:
                // - If ibi_pending: transition to Read, prefill TX FIFO
                // - If hot-join: call client callback, transition to Idle

                // Return to idle state
                // Only set to Idle if handle_event() didn't already change the state
                if self.state.get() == current_state {
                    self.state.set(OperState::Idle);
                }
            }
            OperState::Idle => {
                i3c_verbose!("I");

                // Already idle, just cleanup
                self.flush_tx_fifo(true);
                self.flush_rx_fifo(false);
            }
        }

        // Clear RX buffer data since it's dummy data from an unexpected transfer
        // The buffer should be clean for the next transfer
        self.rx_buffer.map(|buffer| {
            for byte in buffer.iter_mut() {
                *byte = 0;
            }
        });
        self.rx_len.set(0);

        // Start RX DMA for next transfer if in DMA mode
        if self.transfer_mode.get() == TransferMode::Dma {
            self.start_rx_dma_if_needed();
        }

        gpio::set_debug_gpio86(false);
    }

    /// Set RX buffer for receiving writes
    /// This should be called after each transfer completion to provide a new buffer for the next transfer.
    pub fn set_rx_buffer(&self, buffer: &'static mut [u8]) {
        self.rx_buffer.replace(buffer);
        self.rx_len.set(0);

        // Re-enable DMA for next transfer if in DMA mode
        if self.transfer_mode.get() == TransferMode::Dma {
            self.start_rx_dma_transfer();
        }

        // RXPEND interrupt is already enabled in init(), so we're ready to receive data
    }

    /// Set TX buffer for read requests
    /// This should be called before a read request is expected.
    pub fn set_tx_buffer(&self, buffer: &'static mut [u8], len: usize) {
        self.tx_buffer.replace(buffer);
        self.tx_len.set(len);
        self.tx_idx.set(0);

        i3c_verbose!("R {}", actual_len);

        // TX DMA will be started when a read request is received in handle_matched()
    }

    /// Transmit data for read request
    pub fn transmit(
        &self,
        buffer: &'static mut [u8],
        len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        gpio::set_debug_gpio94(true);

        if self.state.get() != OperState::Idle && self.state.get() != OperState::Read {
            return Err((ErrorCode::BUSY, buffer));
        }

        if len > buffer.len() {
            return Err((ErrorCode::SIZE, buffer));
        }

        // Flush TX FIFO to clear any residual data from previous transmissions
        // This prevents old data from being concatenated with new responses
        self.flush_tx_fifo(true);

        // Setup TX buffer and length
        self.set_tx_buffer(buffer, len);

        // If we're already in a read operation, start transmitting
        if self.state.get() == OperState::Read {
            self.handle_tx();
        } else {
            // i3c_verbose!("[I3C Target driver] Send IBI for pending read data");

            // Send IBI to notify controller that data is ready to be read
            let _ = self.send_ibi_with_len(MDB_PENDING_READ_MCTP, 0);

            // IBI send status EVDET will be handled in STOP handler

            // Start TX DMA no matter the IBI was sent or not, this ensures data is ready for read
            if self.start_tx_dma_transfer() == false {
                i3c_debug!("[I3C Target driver] TX DMA could not be started");
            }
        }

        gpio::set_debug_gpio94(false);

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

        // Set state to IBI and mark IBI as pending
        self.state.set(OperState::Ibi);
        self.ibi_pending.set(true);

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
        } else {
            // Write 2-byte length payload (big-endian)
            let len_bytes = len.to_be_bytes();

            // if self.transfer_mode.get() == TransferMode::Fifo {
            // Write first byte (MSB)
            regs.wdatab.set(WDATAB::DATA.val(len_bytes[0] as u32).value);
            // Write second byte (LSB) as end
            regs.wdatabe
                .set(WDATABE::DATA.val(len_bytes[1] as u32).value);
            // }

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
    }

    /// Send In-Band Interrupt (IBI)
    ///
    /// # Arguments
    /// * `mdb` - Mandatory Data Byte to send with the IBI
    /// * `payload` - Optional extended IBI payload (up to 15 additional bytes)
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

        i3c_verbose!(
            "[I3C Target driver] Sending IBI: MDB=0x{:X}, Payload={:?}",
            mdb,
            payload
        );

        // Check if IBI is disabled by the controller
        if regs.status.is_set(STATUS::IBIDIS) {
            i3c_debug!("[I3C Target driver] IBI is disabled by the controller");
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
                } else if self.transfer_mode.get() == TransferMode::Dma {
                    // For DMA mode, we would need to set up a DMA transfer here
                    // TODO: finish it
                    i3c_debug!(
                        "[I3C Target driver] IBI with extended payload in DMA mode not yet implemented"
                    );
                }

                // Set IBIEXT1.CNT to 0
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

    /// Get transfer mode
    pub fn get_transfer_mode(&self) -> TransferMode {
        self.transfer_mode.get()
    }

    /// Flush RX FIFO buffer
    pub fn flush_rx_fifo(&self, enable: bool) {
        if self.transfer_mode.get() == TransferMode::Dma {
            self.disable_rx_dma();
        }
        self.registers.datactrl.modify(DATACTRL::FLUSHFB::SET);
        if self.transfer_mode.get() == TransferMode::Dma && enable {
            self.enable_rx_dma();
        }
    }

    /// Flush TX FIFO buffer
    pub fn flush_tx_fifo(&self, _enable: bool) {
        if self.transfer_mode.get() == TransferMode::Dma {
            i3c_verbose!("=f");

            self.disable_tx_dma();
        }
        self.registers.datactrl.modify(DATACTRL::FLUSHTB::SET);
        // if self.transfer_mode.get() == TransferMode::Dma && enable {
        //     self.enable_tx_dma();
        // }
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
        self.start_rx_dma_transfer();

        i3c_verbose!("[I3C Target driver] HIL set_rx_buffer called");
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
