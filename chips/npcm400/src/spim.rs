// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! NPCM400 SPI Master (SPIM) driver
//!
//! This driver provides support for the NPCM400 SPIM peripheral, which is used
//! for SPI flash memory access with Direct Memory Mapping (DMM) and Normal I/O modes.
//! Based on the ROM SPIM driver implementation.

use core::cell::Cell;
use kernel::hil;
use kernel::utilities::cells::{MapCell, OptionalCell};
use kernel::utilities::leasable_buffer::SubSliceMut;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, ReadOnly, ReadWrite, WriteOnly};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

/// SPIM register base address
const SPIM_BASE_ADDR: usize = 0x4001_7000;

/// SPIM timeout for operations (in loop iterations)
const SPIM_CHK_TIMEOUT: u32 = 10_000;

/// SPIM clock divider
const SPIM_CLK_DIVIDER: u32 = 0x1;

/// System register addresses and bit definitions
const PWDWN_CTL1_ADDR: usize = 0x4000_D008;
const PWDWN_CTL1_BIT: u32 = 1 << 3; // SPIM bit in PWDWN_CTL1
const INT_SPI_CTRL_ADDR: usize = 0x400C_3000;
const INT_SPI_BIT: u32 = 1 << 0;
const INT_SPI_QUAD_BIT: u32 = 1 << 1;
const WP_INT_FL_ADDR: usize = 0x400C_3004;
const WP_INT_FL_BIT: u32 = 1 << 0;

/// SPIM Register definitions
#[repr(C)]
pub struct SpimRegisters {
    /// 0x000: Control and Status Register 0
    ctl0: ReadWrite<u32, Ctl0::Register>,
    /// 0x004: Control and Status Register 1
    ctl1: ReadWrite<u32, Ctl1::Register>,
    _reserved1: u32,
    /// 0x00C: RX Clock Delay Control Register
    rxclkdly: ReadWrite<u32>,
    /// 0x010: Data Receive Register 0
    rx0: ReadOnly<u32>,
    /// 0x014: Data Receive Register 1
    rx1: ReadOnly<u32>,
    /// 0x018: Data Receive Register 2
    rx2: ReadOnly<u32>,
    /// 0x01C: Data Receive Register 3
    rx3: ReadOnly<u32>,
    /// 0x020: Data Transmit Register 0
    tx0: WriteOnly<u32>,
    /// 0x024: Data Transmit Register 1
    tx1: WriteOnly<u32>,
    /// 0x028: Data Transmit Register 2
    tx2: WriteOnly<u32>,
    /// 0x02C: Data Transmit Register 3
    tx3: WriteOnly<u32>,
    /// 0x030: SRAM Memory Address Register
    sramaddr: ReadWrite<u32>,
    /// 0x034: DMA Transfer Byte Count Register
    dmacnt: ReadWrite<u32>,
    /// 0x038: SPI Flash Address Register
    faddr: ReadWrite<u32>,
    _reserved2: [u32; 2],
    /// 0x044: Direct Memory Mapping Mode Control Register
    dmmctl: ReadWrite<u32>,
    /// 0x048: Control Register 2
    ctl2: ReadWrite<u32>,
}

register_bitfields![u32,
    Ctl0 [
        /// Command code
        CMDCODE OFFSET(24) NUMBITS(8) [],
        /// Operation mode
        OPMODE OFFSET(22) NUMBITS(2) [
            NORMAL_IO = 0x0,
            DMA_WRITE = 0x1,
            DMA_READ = 0x2,
            DMM = 0x3
        ],
        /// Bit mode
        BITMODE OFFSET(20) NUMBITS(2) [
            STANDARD = 0x0,
            DUAL = 0x1,
            QUAD = 0x2
        ],
        /// Suspend interval
        SUSPITV OFFSET(16) NUMBITS(4) [],
        /// Quad I/O direction
        QDIODIR OFFSET(15) NUMBITS(1) [],
        /// Burst number
        BURSTNUM OFFSET(13) NUMBITS(2) [
            BURST_1 = 0x0,
            BURST_2 = 0x1,
            BURST_3 = 0x2,
            BURST_4 = 0x3
        ],
        /// Data width
        DWIDTH OFFSET(8) NUMBITS(5) [
            WIDTH_8 = 0x7,
            WIDTH_16 = 0xF,
            WIDTH_24 = 0x17,
            WIDTH_32 = 0x1F
        ],
        /// Interrupt flag
        IF OFFSET(7) NUMBITS(1) [],
        /// Interrupt enable
        IEN OFFSET(6) NUMBITS(1) [],
        /// 4-byte address enable
        B4ADDREN OFFSET(5) NUMBITS(1) [],
        /// Cipher off
        CIPHOFF OFFSET(0) NUMBITS(1) []
    ],

    Ctl1 [
        /// Clock divider
        DIVIDER OFFSET(16) NUMBITS(16) [],
        /// Idle time
        IDLE_TIME OFFSET(8) NUMBITS(4) [],
        /// Slave select active polarity
        SSACTPOL OFFSET(5) NUMBITS(1) [],
        /// Slave select
        SS OFFSET(4) NUMBITS(1) [],
        /// Clock divider invalid
        CDINVAL OFFSET(3) NUMBITS(1) [],
        /// Cache off
        CACHEOFF OFFSET(1) NUMBITS(1) [],
        /// SPI master enable
        SPIMEN OFFSET(0) NUMBITS(1) []
    ]
];

/// SPIM chip select definitions
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChipSelect {
    /// CS0 - internal flash
    Internal,
}

impl ChipSelect {
    #[inline(always)]
    fn sw_index(&self) -> u8 {
        match self {
            ChipSelect::Internal => 0,
        }
    }
}

/// SPIM read modes
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReadMode {
    Normal,
    Fast,
    FastDual,
    Quad,
}

impl ReadMode {
    fn to_spi_cmd(&self) -> u8 {
        match self {
            ReadMode::Normal => 0x03,      // SPI_NOR_CMD_READ
            ReadMode::Fast => 0x0B,        // SPI_NOR_CMD_READ_FAST
            ReadMode::FastDual => 0xBB,    // SPI_NOR_CMD_2READ
            ReadMode::Quad => 0xEB,        // SPI_NOR_CMD_4READ
        }
    }
}

/// SPIM operation flags
#[derive(Debug, Copy, Clone)]
pub struct OperationFlags {
    pub enable_write_protect: bool,
    pub lock_transceive: bool,
}

impl Default for OperationFlags {
    fn default() -> Self {
        Self {
            enable_write_protect: false,
            lock_transceive: false,
        }
    }
}

/// Transceive operation flags
pub const TRANSCEIVE_ACCESS_WRITE: u32 = 1 << 0;
pub const TRANSCEIVE_ACCESS_READ: u32 = 1 << 1;
pub const TRANSCEIVE_ACCESS_ADDR: u32 = 1 << 2;

/// Flash operation client callback trait
pub trait FlashClient {
    /// Called when a flash erase operation completes
    fn erase_done(&self, result: Result<(), ErrorCode>);

    /// Called when a flash write (program) operation completes
    fn write_done(&self, result: Result<(), ErrorCode>);

    /// Called when a flash read operation completes
    fn read_done(&self, buffer: &[u8], result: Result<(), ErrorCode>);
}

/// Flash operation state
#[derive(Copy, Clone, Debug, PartialEq)]
enum FlashOpState {
    Idle,
    WriteEnable,
    EraseCommand,
    ProgramCommand,
    WaitReady,
    ReadStatus,
    Complete,
}

/// Flash operation type
#[derive(Copy, Clone, Debug, PartialEq)]
enum FlashOpType {
    None,
    Erase,
    Program,
}

/// SPIM configuration structure
#[derive(Debug, Copy, Clone)]
pub struct Config {
    pub chip_select: ChipSelect,
    pub read_mode: ReadMode,
    pub enter_4ba: bool, // 4-byte address mode
}

impl Default for Config {
    fn default() -> Self {
        Self {
            chip_select: ChipSelect::Internal,
            read_mode: ReadMode::Normal,
            enter_4ba: false,
        }
    }
}

/// Transceive configuration for Normal I/O operations
#[derive(Debug)]
pub struct TransceiveConfig<'a> {
    pub opcode: u8,
    pub address: Option<u32>,
    pub tx_data: Option<&'a [u8]>,
    pub rx_data: Option<&'a mut [u8]>,
}

/// SPIM driver structure
pub struct Spim<'a> {
    registers: StaticRef<SpimRegisters>,
    current_config: Cell<Option<Config>>,
    current_operation: Cell<OperationFlags>,
    sw_cs_mask: Cell<u8>,
    // SPI HIL support
    client: OptionalCell<&'a dyn hil::spi::SpiMasterClient>,
    busy: Cell<bool>,
    tx_buf: MapCell<SubSliceMut<'static, u8>>,
    rx_buf: MapCell<SubSliceMut<'static, u8>>,
    transfer_len: Cell<usize>,
    current_chip_select: Cell<ChipSelect>,
    // SPI configuration state
    rate: Cell<u32>,
    polarity: Cell<hil::spi::ClockPolarity>,
    phase: Cell<hil::spi::ClockPhase>,
    data_order: Cell<hil::spi::DataOrder>,
    // Flash operation support
    flash_client: OptionalCell<&'a dyn FlashClient>,
    flash_op_state: Cell<FlashOpState>,
    flash_op_type: Cell<FlashOpType>,
    flash_op_address: Cell<u32>,
    flash_buffer: MapCell<&'static mut [u8]>,
}

impl<'a> Spim<'a> {
    /// Create a new SPIM driver instance
    pub const fn new() -> Self {
        Self {
            registers: unsafe { StaticRef::new(SPIM_BASE_ADDR as *const SpimRegisters) },
            current_config: Cell::new(None),
            current_operation: Cell::new(OperationFlags {
                enable_write_protect: false,
                lock_transceive: false,
            }),
            sw_cs_mask: Cell::new(0),
            client: OptionalCell::empty(),
            busy: Cell::new(false),
            tx_buf: MapCell::empty(),
            rx_buf: MapCell::empty(),
            transfer_len: Cell::new(0),
            current_chip_select: Cell::new(ChipSelect::Internal),
            rate: Cell::new(8_000_000), // Default 8MHz
            polarity: Cell::new(hil::spi::ClockPolarity::IdleLow),
            phase: Cell::new(hil::spi::ClockPhase::SampleLeading),
            data_order: Cell::new(hil::spi::DataOrder::MSBFirst),
            flash_client: OptionalCell::empty(),
            flash_op_state: Cell::new(FlashOpState::Idle),
            flash_op_type: Cell::new(FlashOpType::None),
            flash_op_address: Cell::new(0),
            flash_buffer: MapCell::empty(),
        }
    }

    /// Set the flash operation client
    pub fn set_flash_client(&self, client: &'a dyn FlashClient) {
        self.flash_client.set(client);
    }

    /// Initialize the SPIM controller
    pub fn init(&self) -> Result<(), ErrorCode> {
        self.init_power_control()?;
        self.init_pinmux()?;
        self.init_clock()?;
        self.enable_cache();
        Ok(())
    }

    /// Initialize power down control
    fn init_power_control(&self) -> Result<(), ErrorCode> {
        unsafe {
            let reg = PWDWN_CTL1_ADDR as *mut u32;
            reg.write_volatile(reg.read_volatile() & !PWDWN_CTL1_BIT);
        }
        Ok(())
    }

    /// Initialize pinmux for internal flash
    fn init_pinmux(&self) -> Result<(), ErrorCode> {
        unsafe {
            // INT_SPI=1 and INT_SPI_QUAD=1
            let reg = INT_SPI_CTRL_ADDR as *mut u32;
            reg.write_volatile(reg.read_volatile() | INT_SPI_BIT | INT_SPI_QUAD_BIT);

            // WP_INT_FL=0 (disable write protect by default)
            let reg = WP_INT_FL_ADDR as *mut u32;
            reg.write_volatile(reg.read_volatile() & !WP_INT_FL_BIT);
        }
        Ok(())
    }

    /// Initialize clock divider
    fn init_clock(&self) -> Result<(), ErrorCode> {
        // Assume source clock > 50MHz, set divider
        self.registers.ctl1.modify(Ctl1::DIVIDER.val(SPIM_CLK_DIVIDER));
        Ok(())
    }

    /// Enable cache
    fn enable_cache(&self) {
        self.registers.ctl1.modify(Ctl1::CACHEOFF::CLEAR);
    }

    /// Invalidate cache
    fn invalidate_cache(&self) {
        self.registers.ctl1.modify(Ctl1::CDINVAL::SET);

        // Wait for cache invalidation to complete
        while self.registers.ctl1.is_set(Ctl1::CDINVAL) {
            // Busy wait
        }
    }

    /// Configure the SPIM for specific operation
    pub fn configure(
        &self,
        config: &Config,
        operation: OperationFlags,
    ) -> Result<(), ErrorCode> {
        // Check if config is different from current config
        let config_changed = self.current_config.get().map_or(true, |current| {
            current.chip_select != config.chip_select
                || current.read_mode != config.read_mode
                || current.enter_4ba != config.enter_4ba
        });

        if config_changed {
            self.current_config.set(Some(*config));
            self.sw_cs_mask.set(config.chip_select.sw_index());
            self.config_dmm_mode(config)?; // DMM mode for direct memory mapping
        }

        self.current_operation.set(operation);
        if operation.enable_write_protect {
            self.set_write_protect()?;
        }
        Ok(())
    }

    /// Configure Normal I/O mode
    fn config_normal_mode(&self, _config: &Config) -> Result<(), ErrorCode> {
        let mut ctl0 = self.registers.ctl0.get();

        // Set normal I/O mode
        ctl0 &= !(0x3 << 22); // Clear OPMODE
        ctl0 |= 0x0 << 22; // OPMODE_NORMAL_IO

        // Set standard bit mode
        ctl0 &= !(0x3 << 20); // Clear BITMODE
        ctl0 |= 0x0 << 20; // BITMODE_STANDARD

        // Set 8-bit data width
        ctl0 &= !(0x1F << 8); // Clear DWIDTH
        ctl0 |= 0x7 << 8; // DWIDTH_8

        // Set burst number to 1
        ctl0 &= !(0x3 << 13); // Clear BURSTNUM
        ctl0 |= 0x0 << 13; // BURSTNUM_1

        self.registers.ctl0.set(ctl0);
        Ok(())
    }

    /// Configure Direct Memory Mapping (DMM) mode
    fn config_dmm_mode(&self, config: &Config) -> Result<(), ErrorCode> {
        let mut ctl0 = self.registers.ctl0.get();

        // Set DMM mode
        ctl0 &= !(0x3 << 22); // Clear OPMODE
        ctl0 |= 0x3 << 22; // OPMODE_DMM

        // Set command code based on read mode
        let cmdcode = config.read_mode.to_spi_cmd();
        ctl0 &= !(0xFF << 24); // Clear CMDCODE
        ctl0 |= (cmdcode as u32) << 24;

        // Configure 4-byte address mode
        if config.enter_4ba {
            ctl0 |= 1 << 5; // B4ADDREN
        } else {
            ctl0 &= !(1 << 5);
        }

        self.registers.ctl0.set(ctl0);
        Ok(())
    }

    /// Set chip select level (high/low)
    fn set_cs_level(&self, _sw_cs: u8, level: bool) {
        if level {
            self.registers.ctl1.modify(Ctl1::SS::SET);
        } else {
            self.registers.ctl1.modify(Ctl1::SS::CLEAR);
        }
    }

    /// Write a single byte in normal mode
    fn normal_write_byte(&self, data: u8) -> Result<(), ErrorCode> {
        // Set output direction
        self.registers.ctl0.modify(Ctl0::QDIODIR::SET);

        // Fill data
        self.registers.tx0.set(data as u32);

        // Execute transaction
        self.registers.ctl1.modify(Ctl1::SPIMEN::SET);

        // Wait for completion
        self.wait_spi_complete()?;

        // Clear interrupt flag
        self.registers.ctl0.modify(Ctl0::IF::SET);

        Ok(())
    }

    /// Read a single byte in normal mode
    fn normal_read_byte(&self) -> Result<u8, ErrorCode> {
        // Set input direction
        self.registers.ctl0.modify(Ctl0::QDIODIR::CLEAR);

        // Execute transaction
        self.registers.ctl1.modify(Ctl1::SPIMEN::SET);

        // Wait for completion
        self.wait_spi_complete()?;

        // Clear interrupt flag
        self.registers.ctl0.modify(Ctl0::IF::SET);

        // Read data
        Ok((self.registers.rx0.get() & 0xFF) as u8)
    }

    /// Wait for SPI transaction completion
    fn wait_spi_complete(&self) -> Result<(), ErrorCode> {
        let mut timeout = SPIM_CHK_TIMEOUT;

        // Wait while SPIMEN is set (transaction running)
        while timeout > 0 {
            if !self.registers.ctl1.is_set(Ctl1::SPIMEN) {
                break;
            }
            timeout -= 1;
        }

        if timeout == 0 {
            return Err(ErrorCode::FAIL);
        }

        Ok(())
    }

    /// Wait for flash to be ready (WIP bit clear)
    fn wait_flash_ready(&self, sw_cs: u8) -> Result<(), ErrorCode> {
        let mut timeout = 100000; // Increased timeout for erase operations

        while timeout > 0 {
            self.set_cs_level(sw_cs, false);
            self.normal_write_byte(0x05)?; // Read Status Register command
            let status = self.normal_read_byte()?;
            self.set_cs_level(sw_cs, true);

            if (status & 0x01) == 0 {
                // WIP bit is clear
                break;
            }
            timeout -= 1;
        }

        if timeout == 0 {
            return Err(ErrorCode::FAIL);
        }

        Ok(())
    }

    /// Set write protect
    fn set_write_protect(&self) -> Result<(), ErrorCode> {
        unsafe {
            let reg = WP_INT_FL_ADDR as *mut u32;
            reg.write_volatile(reg.read_volatile() | WP_INT_FL_BIT);
        }
        Ok(())
    }

    /// Perform Normal I/O transceive operation
    pub fn normal_transceive(
        &self,
        config: &mut TransceiveConfig,
        flags: u32,
    ) -> Result<(), ErrorCode> {
        if self.current_operation.get().lock_transceive {
            return Err(ErrorCode::BUSY);
        }

        let current_config = self.current_config.get().ok_or(ErrorCode::FAIL)?;

        // Save current CTL0 setting
        let saved_ctl0 = self.registers.ctl0.get();

        // Configure normal mode
        self.config_normal_mode(&current_config)?;

        let sw_cs = current_config.chip_select.sw_index();

        // Perform transaction
        let result = self.normal_transceive_inner(config, flags, &current_config, sw_cs);

        // Restore CTL0 setting
        self.registers.ctl0.set(saved_ctl0);

        // Invalidate cache after normal I/O mode
        self.invalidate_cache();

        result
    }

    /// Inner normal transceive operation
    fn normal_transceive_inner(
        &self,
        config: &mut TransceiveConfig,
        flags: u32,
        current_config: &Config,
        sw_cs: u8,
    ) -> Result<(), ErrorCode> {
        // Assert chip select
        self.set_cs_level(sw_cs, false);

        // Transmit opcode
        self.normal_write_byte(config.opcode)?;

        // Address phase
        if (flags & TRANSCEIVE_ACCESS_ADDR) != 0 {
            let addr = config
                .address
                .ok_or(ErrorCode::INVAL)?;
            let start = if current_config.enter_4ba { 0 } else { 1 };
            for i in start..4 {
                let b = ((addr >> (8 * (3 - i))) & 0xFF) as u8;
                self.normal_write_byte(b)?;
            }
        }

        // Write phase
        if (flags & TRANSCEIVE_ACCESS_WRITE) != 0 {
            if let Some(tx) = config.tx_data {
                for &b in tx {
                    self.normal_write_byte(b)?;
                }
            } else {
                return Err(ErrorCode::INVAL);
            }
        }

        // Read phase
        if (flags & TRANSCEIVE_ACCESS_READ) != 0 {
            if let Some(rx) = config.rx_data.as_mut() {
                for byte in rx.iter_mut() {
                    *byte = self.normal_read_byte()?;
                }
            } else {
                return Err(ErrorCode::INVAL);
            }
        }

        // De-assert chip select
        self.set_cs_level(sw_cs, true);

        // Wait for flash ready if it's a write operation (but not WREN)
        if (flags & TRANSCEIVE_ACCESS_READ) == 0 && config.opcode != 0x06 {
            self.wait_flash_ready(sw_cs)?;
        }

        Ok(())
    }

    /// Read flash status register
    pub fn read_status(&self) -> Result<u8, ErrorCode> {
        let mut status = [0u8; 1];
        let mut config = TransceiveConfig {
            opcode: 0x05, // Read Status Register command
            address: None,
            tx_data: None,
            rx_data: Some(&mut status),
        };

        self.normal_transceive(&mut config, TRANSCEIVE_ACCESS_READ)?;
        Ok(status[0])
    }

    /// Write enable command
    pub fn write_enable(&self) -> Result<(), ErrorCode> {
        let mut config = TransceiveConfig {
            opcode: 0x06, // Write Enable command
            address: None,
            tx_data: None,
            rx_data: None,
        };

        self.normal_transceive(&mut config, 0)
    }

    /// Sector erase command
    pub fn sector_erase(&self, address: u32) -> Result<(), ErrorCode> {
        let mut config = TransceiveConfig {
            opcode: 0x20, // Sector Erase command
            address: Some(address),
            tx_data: None,
            rx_data: None,
        };

        self.normal_transceive(&mut config, TRANSCEIVE_ACCESS_ADDR)
    }

    /// Page program command
    pub fn page_program(&self, address: u32, data: &[u8]) -> Result<(), ErrorCode> {
        let mut config = TransceiveConfig {
            opcode: 0x02, // Page Program command
            address: Some(address),
            tx_data: Some(data),
            rx_data: None,
        };

        self.normal_transceive(
            &mut config,
            TRANSCEIVE_ACCESS_ADDR | TRANSCEIVE_ACCESS_WRITE,
        )
    }

    // =============================
    // Async Flash Operations (HIL)
    // =============================

    /// Async sector erase - erases a 4KB sector
    /// Callback will be invoked via FlashClient::erase_done when complete
    /// No buffer needed - erase command is small and handled internally
    ///
    /// WARNING: This function uses synchronous polling and will BLOCK the kernel!
    /// Do NOT erase sectors that contain executable code or the kernel will crash.
    /// TODO: Convert to truly async operation with timer-based polling.
    pub fn async_sector_erase(&self, address: u32) -> Result<(), ErrorCode> {
        if self.flash_op_state.get() != FlashOpState::Idle {
            return Err(ErrorCode::BUSY);
        }

        if self.flash_client.is_none() {
            return Err(ErrorCode::RESERVE);
        }

        // Store operation parameters
        self.flash_op_type.set(FlashOpType::Erase);
        self.flash_op_address.set(address);

        // Start with Write Enable command
        self.start_write_enable_for_erase()
    }

    /// Async page program - programs up to 256 bytes
    /// Callback will be invoked via FlashClient::write_done when complete
    pub fn async_page_program(
        &self,
        address: u32,
        buffer: &'static mut [u8],
        len: usize,
    ) -> Result<(), ErrorCode> {
        if self.flash_op_state.get() != FlashOpState::Idle {
            return Err(ErrorCode::BUSY);
        }

        if self.flash_client.is_none() {
            return Err(ErrorCode::RESERVE);
        }

        if buffer.len() < len || len > 256 {
            return Err(ErrorCode::SIZE);
        }

        // Store operation parameters
        self.flash_op_type.set(FlashOpType::Program);
        self.flash_op_address.set(address);
        self.flash_buffer.replace(buffer);

        // Start with Write Enable command
        self.start_write_enable_for_program(len)
    }

    /// Start Write Enable command (for erase)
    fn start_write_enable_for_erase(&self) -> Result<(), ErrorCode> {
        self.flash_op_state.set(FlashOpState::WriteEnable);

        // Send WREN via normal I/O
        let current_config = self.current_config.get().unwrap_or_default();
        let sw_cs = current_config.chip_select.sw_index();

        // Save and configure for normal mode
        let saved_ctl0 = self.registers.ctl0.get();
        self.config_normal_mode(&current_config).ok();

        // Send WREN command (0x06)
        self.set_cs_level(sw_cs, false);
        let result = self.normal_write_byte(0x06);
        self.set_cs_level(sw_cs, true);

        // Restore mode
        self.registers.ctl0.set(saved_ctl0);

        result?;

        // Continue to erase command
        self.continue_erase_operation()
    }

    /// Continue with erase command after WREN
    fn continue_erase_operation(&self) -> Result<(), ErrorCode> {
        let address = self.flash_op_address.get();
        self.flash_op_state.set(FlashOpState::EraseCommand);

        // Send erase command directly (no buffer needed)
        let current_config = self.current_config.get().unwrap_or_default();
        let sw_cs = current_config.chip_select.sw_index();

        let saved_ctl0 = self.registers.ctl0.get();
        self.config_normal_mode(&current_config).ok();

        self.set_cs_level(sw_cs, false);

        // Send: Sector Erase opcode + 3-byte address
        self.normal_write_byte(0x20)?; // Sector Erase (4KB)
        self.normal_write_byte(((address >> 16) & 0xFF) as u8)?;
        self.normal_write_byte(((address >> 8) & 0xFF) as u8)?;
        self.normal_write_byte((address & 0xFF) as u8)?;

        self.set_cs_level(sw_cs, true);

        self.registers.ctl0.set(saved_ctl0);

        // Start polling for completion
        self.flash_op_state.set(FlashOpState::WaitReady);
        self.poll_flash_ready()
    }

    /// Start Write Enable command (for program)
    fn start_write_enable_for_program(&self, data_len: usize) -> Result<(), ErrorCode> {
        self.flash_buffer.map_or(Err(ErrorCode::FAIL), |buf| {
            // Save data length in first position (will be overwritten with command)
            let address = self.flash_op_address.get();

            // Send WREN
            let current_config = self.current_config.get().unwrap_or_default();
            let sw_cs = current_config.chip_select.sw_index();

            let saved_ctl0 = self.registers.ctl0.get();
            self.config_normal_mode(&current_config).ok();

            self.set_cs_level(sw_cs, false);
            self.normal_write_byte(0x06)?; // WREN
            self.set_cs_level(sw_cs, true);

            self.registers.ctl0.set(saved_ctl0);

            self.flash_op_state.set(FlashOpState::ProgramCommand);

            // Continue to program command
            self.continue_program_operation(address, data_len)
        })
    }

    /// Continue with program command after WREN
    fn continue_program_operation(&self, address: u32, data_len: usize) -> Result<(), ErrorCode> {
        self.flash_buffer.map_or(Err(ErrorCode::FAIL), |buf| {
            let current_config = self.current_config.get().unwrap_or_default();
            let sw_cs = current_config.chip_select.sw_index();

            let saved_ctl0 = self.registers.ctl0.get();
            self.config_normal_mode(&current_config).ok();

            self.set_cs_level(sw_cs, false);

            // Send Page Program command
            self.normal_write_byte(0x02)?;

            // Send address (3 bytes)
            self.normal_write_byte(((address >> 16) & 0xFF) as u8)?;
            self.normal_write_byte(((address >> 8) & 0xFF) as u8)?;
            self.normal_write_byte((address & 0xFF) as u8)?;

            // Send data
            for i in 0..data_len {
                self.normal_write_byte(buf[i])?;
            }

            self.set_cs_level(sw_cs, true);

            self.registers.ctl0.set(saved_ctl0);

            // Start polling for completion
            self.flash_op_state.set(FlashOpState::WaitReady);
            self.poll_flash_ready()
        })
    }

    /// Poll flash ready status with timeout
    /// WARNING: This is synchronous polling and will block!
    /// TODO: Convert to timer-based async polling
    fn poll_flash_ready(&self) -> Result<(), ErrorCode> {
        let current_config = self.current_config.get().unwrap_or_default();
        let sw_cs = current_config.chip_select.sw_index();

        let saved_ctl0 = self.registers.ctl0.get();

        // Poll with timeout to prevent infinite loop/stack overflow
        const MAX_POLL_ATTEMPTS: u32 = 100000;
        let mut attempts = 0;

        loop {
            // Switch to Normal I/O mode only for status read
            // Must restore DMM mode after each poll so CPU can fetch instructions!
            self.config_normal_mode(&current_config).ok();

            self.set_cs_level(sw_cs, false);
            self.normal_write_byte(0x05)?; // Read Status Register
            let status = self.normal_read_byte()?;
            self.set_cs_level(sw_cs, true);

            // Restore DMM mode immediately so CPU can continue executing
            self.registers.ctl0.set(saved_ctl0);

            if (status & 0x01) == 0 {
                // WIP bit is clear - operation complete
                return self.flash_operation_complete(Ok(()));
            }

            attempts += 1;
            if attempts >= MAX_POLL_ATTEMPTS {
                // Timeout
                return self.flash_operation_complete(Err(ErrorCode::FAIL));
            }

            // Small delay between polls to reduce bus contention
            for _ in 0..100 {
                // Busy wait
            }
        }
    }

    /// Called when flash operation completes
    fn flash_operation_complete(&self, result: Result<(), ErrorCode>) -> Result<(), ErrorCode> {
        let op_type = self.flash_op_type.get();
        self.flash_op_state.set(FlashOpState::Idle);
        self.flash_op_type.set(FlashOpType::None);

        // Call appropriate callback based on operation type
        self.flash_client.map(|client| {
            match op_type {
                FlashOpType::Erase => {
                    client.erase_done(result);
                }
                FlashOpType::Program => {
                    client.write_done(result);
                }
                FlashOpType::None => {
                    // Shouldn't happen
                }
            }
        });

        // Return buffer if one was used (program operation)
        self.flash_buffer.take();
        Ok(())
    }

    /// Handle interrupt - called when SPI transaction completes
    pub fn handle_interrupt(&self) {
        // Clear the interrupt flag
        self.registers.ctl0.modify(Ctl0::IF::SET);

        // Mark as not busy
        self.busy.set(false);

        // Call the client callback
        self.client.map(|client| {
            self.tx_buf.take().map(|tx_buf| {
                let len = self.transfer_len.get();
                client.read_write_done(tx_buf, self.rx_buf.take(), Ok(len));
            });
        });
    }
}

// ======================
// SPI HIL Implementation
// ======================

/// Implementation of the SpiMaster trait for SPIM
impl<'a> hil::spi::SpiMaster<'a> for Spim<'a> {
    type ChipSelect = ChipSelect;

    fn set_client(&self, client: &'a dyn hil::spi::SpiMasterClient) {
        self.client.set(client);
    }

    fn init(&self) -> Result<(), ErrorCode> {
        // Already implemented as a separate init function
        self.init_power_control()?;
        self.init_pinmux()?;
        self.init_clock()?;
        self.enable_cache();
        Ok(())
    }

    fn is_busy(&self) -> bool {
        self.busy.get()
    }

    fn read_write_bytes(
        &self,
        tx_buf: SubSliceMut<'static, u8>,
        rx_buf: Option<SubSliceMut<'static, u8>>,
    ) -> Result<
        (),
        (
            ErrorCode,
            SubSliceMut<'static, u8>,
            Option<SubSliceMut<'static, u8>>,
        ),
    > {
        // Check if busy
        if self.busy.get() {
            return Err((ErrorCode::BUSY, tx_buf, rx_buf));
        }

        // Check for valid buffers
        if tx_buf.len() == 0 {
            return Err((ErrorCode::INVAL, tx_buf, rx_buf));
        }

        // Check if client is set
        if self.client.is_none() {
            return Err((ErrorCode::RESERVE, tx_buf, rx_buf));
        }

        // For NPCM400, we'll use Normal I/O mode for standard SPI operations
        // Configure normal mode
        let config = Config {
            chip_select: self.current_chip_select.get(),
            read_mode: ReadMode::Normal,
            enter_4ba: false,
        };

        if let Err(_) = self.config_normal_mode(&config) {
            return Err((ErrorCode::FAIL, tx_buf, rx_buf));
        }

        // Set busy flag
        self.busy.set(true);

        // Assert chip select
        let sw_cs = self.current_chip_select.get().sw_index();
        self.set_cs_level(sw_cs, false);

        // Calculate transfer length
        let tx_len = tx_buf.len();
        let rx_len = rx_buf.as_ref().map_or(0, |buf| buf.len());
        let transfer_len = core::cmp::min(tx_len, core::cmp::max(tx_len, rx_len));
        self.transfer_len.set(transfer_len);

        // Store buffers
        self.tx_buf.replace(tx_buf);
        if let Some(rx) = rx_buf {
            self.rx_buf.replace(rx);
        }

        // Perform synchronous transfer for now
        // TODO: Convert to async with interrupt-driven operation
        let result = self.perform_sync_transfer(transfer_len);

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                self.busy.set(false);
                let tx = self.tx_buf.take().unwrap();
                let rx = self.rx_buf.take();
                Err((e, tx, rx))
            }
        }
    }

    fn write_byte(&self, _val: u8) -> Result<(), ErrorCode> {
        Err(ErrorCode::NOSUPPORT)
    }

    fn read_byte(&self) -> Result<u8, ErrorCode> {
        Err(ErrorCode::NOSUPPORT)
    }

    fn read_write_byte(&self, _val: u8) -> Result<u8, ErrorCode> {
        Err(ErrorCode::NOSUPPORT)
    }

    fn specify_chip_select(&self, cs: Self::ChipSelect) -> Result<(), ErrorCode> {
        self.current_chip_select.set(cs);
        self.sw_cs_mask.set(cs.sw_index());
        Ok(())
    }

    fn set_rate(&self, rate: u32) -> Result<u32, ErrorCode> {
        // NPCM400 SPIM uses a simple clock divider
        // Assume source clock is 50MHz
        const SRC_CLOCK: u32 = 50_000_000;

        if rate == 0 {
            return Err(ErrorCode::INVAL);
        }

        // Calculate divider: SPI_CLK = SRC_CLK / (2 * (DIVIDER + 1))
        // So DIVIDER = (SRC_CLK / (2 * rate)) - 1
        let divider = (SRC_CLOCK / (2 * rate)).saturating_sub(1);
        let divider = core::cmp::min(divider, 0xFFFF); // 16-bit max

        // Calculate actual rate
        let actual_rate = SRC_CLOCK / (2 * (divider + 1));

        // Set the divider
        self.registers.ctl1.modify(Ctl1::DIVIDER.val(divider));
        self.rate.set(actual_rate);

        Ok(actual_rate)
    }

    fn get_rate(&self) -> u32 {
        self.rate.get()
    }

    fn set_polarity(&self, polarity: hil::spi::ClockPolarity) -> Result<(), ErrorCode> {
        if self.busy.get() {
            return Err(ErrorCode::BUSY);
        }

        // NPCM400 SPIM doesn't have explicit polarity control in the same way as standard SPI
        // It uses CPOL/CPHA through the CONFIG bits, which we'll manage through phase
        self.polarity.set(polarity);
        Ok(())
    }

    fn get_polarity(&self) -> hil::spi::ClockPolarity {
        self.polarity.get()
    }

    fn set_phase(&self, phase: hil::spi::ClockPhase) -> Result<(), ErrorCode> {
        if self.busy.get() {
            return Err(ErrorCode::BUSY);
        }

        // NPCM400 SPIM doesn't have standard CPHA control
        // Store the phase for future reference
        self.phase.set(phase);
        Ok(())
    }

    fn get_phase(&self) -> hil::spi::ClockPhase {
        self.phase.get()
    }

    fn hold_low(&self) {
        // Not implemented
    }

    fn release_low(&self) {
        // Not implemented
    }
}

impl<'a> Spim<'a> {
    /// Perform a synchronous SPI transfer
    /// This is a temporary implementation - should be converted to async
    fn perform_sync_transfer(&self, len: usize) -> Result<(), ErrorCode> {
        // Transmit and receive data byte by byte
        self.tx_buf.map(|tx_buf| {
            for i in 0..len {
                if i < tx_buf.len() {
                    // Write byte
                    if let Err(e) = self.normal_write_byte(tx_buf[i]) {
                        return Err(e);
                    }
                }
            }
            Ok(())
        }).unwrap_or(Err(ErrorCode::FAIL))?;

        // Read data if rx buffer is provided
        self.rx_buf.map(|rx_buf| {
            for i in 0..core::cmp::min(len, rx_buf.len()) {
                match self.normal_read_byte() {
                    Ok(byte) => rx_buf[i] = byte,
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }).transpose()?;

        // De-assert chip select
        let sw_cs = self.current_chip_select.get().sw_index();
        self.set_cs_level(sw_cs, true);

        // Trigger completion callback
        self.handle_interrupt();

        Ok(())
    }
}
