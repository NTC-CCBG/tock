// Licensed under the Apache-2.0 license.
//
// NPCM400 Flash Interface Unit (FIU) driver for Tock kernel
// Based on ROM FIU driver and Zephyr NPCM FIU QSPI driver

use core::cell::Cell;
use kernel::deferred_call::{DeferredCall, DeferredCallClient};
use kernel::hil;
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::cells::TakeCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, ReadOnly, ReadWrite};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

/// FIU register base addresses
const FIU0_BASE: usize = 0x40020000; // FIU Core
const FIU1_BASE: usize = 0x40021000; // FIU Host

/// FIU Register definitions
#[repr(C)]
pub struct FiuRegisters {
    _reserved0: u8,                                       // 0x000
    burst_cfg: ReadWrite<u8, BurstCfg::Register>,         // 0x001
    resp_cfg: ReadWrite<u8, RespCfg::Register>,           // 0x002
    _reserved1: [u8; 17],                                 // 0x003-0x013
    spi_fl_cfg: ReadWrite<u8, SpiFlCfg::Register>,        // 0x014
    _reserved2: u8,                                       // 0x015
    uma_code: ReadWrite<u8, UmaCode::Register>,           // 0x016
    uma_ab0: ReadWrite<u8, UmaAb0::Register>,             // 0x017
    uma_ab1: ReadWrite<u8, UmaAb1::Register>,             // 0x018
    uma_ab2: ReadWrite<u8, UmaAb2::Register>,             // 0x019
    uma_db0: ReadWrite<u8, UmaDb0::Register>,             // 0x01A
    uma_db1: ReadWrite<u8, UmaDb1::Register>,             // 0x01B
    uma_db2: ReadWrite<u8, UmaDb2::Register>,             // 0x01C
    uma_db3: ReadWrite<u8, UmaDb3::Register>,             // 0x01D
    uma_cts: ReadWrite<u8, UmaCts::Register>,             // 0x01E
    uma_ects: ReadWrite<u8, UmaEcts::Register>,           // 0x01F
    uma_db0_3: ReadWrite<u32>,                            // 0x020
    _reserved3: [u8; 2],                                  // 0x024-0x025
    crccon: ReadWrite<u8>,                                // 0x026
    crcent: ReadWrite<u8>,                                // 0x027
    crcrslt: ReadOnly<u32>,                               // 0x028
    _reserved4: [u8; 2],                                  // 0x02C-0x02D
    rd_cmd_back: ReadWrite<u8>,                           // 0x02E
    _reserved5: u8,                                       // 0x02F
    rd_cmd_pvt: ReadWrite<u8>,                            // 0x030
    rd_cmd_shd: ReadWrite<u8>,                            // 0x031
    _reserved6: u8,                                       // 0x032
    fiu_ext_cfg: ReadWrite<u8>,                           // 0x033
    uma_ab0_3: ReadWrite<u32>,                            // 0x034
    _reserved7: [u8; 4],                                  // 0x038-0x03B
    set_cmd_en: ReadWrite<u8>,                            // 0x03C
    addr_4b_en: ReadWrite<u8, Addr4bEn::Register>,        // 0x03D
    _reserved8: [u8; 3],                                  // 0x03E-0x040
    mi_cnt_thrsh: ReadWrite<u8>,                          // 0x041
    fiu_msr_sts: ReadOnly<u8, FiuMsrSts::Register>,       // 0x042
    fiu_msr_ie_cfg: ReadWrite<u8, FiuMsrIeCfg::Register>, // 0x043
    q_p_en: ReadWrite<u8>,                                // 0x044
    _reserved9: [u8; 3],                                  // 0x045-0x047
    ext_db_cfg: ReadWrite<u8>,                            // 0x048
    direct_wr_cfg: ReadWrite<u8>,                         // 0x049
    _reserved10: [u8; 6],                                 // 0x04A-0x04F
    ext_db_f_0: [ReadWrite<u8>; 16],                      // 0x050-0x05F
}

register_bitfields![u8,
    BurstCfg [
        R_BURST OFFSET(0) NUMBITS(2) [
            BURST_1B = 0,
            BURST_4B = 1,
            BURST_8B = 2,
            BURST_16B = 3
        ],
        SLAVE OFFSET(2) NUMBITS(1) [],
        UNLIM_BURST OFFSET(3) NUMBITS(1) [],
        SPI_WR_EN OFFSET(7) NUMBITS(1) []
    ],
    RespCfg [
        QUAD_EN OFFSET(3) NUMBITS(1) []  // Corrected: bit 3, not bit 0
    ],
    SpiFlCfg [
        RD_MODE OFFSET(6) NUMBITS(2) [  // Corrected: bits 6-7, not bits 1-2
            NORMAL = 0,
            FAST = 1,
            FAST_DUAL = 3
        ]
    ],
    UmaCts [
        D_SIZE OFFSET(0) NUMBITS(3) [],
        A_SIZE OFFSET(3) NUMBITS(1) [],
        C_SIZE OFFSET(4) NUMBITS(1) [],
        RD_WR OFFSET(5) NUMBITS(1) [],
        DEV_NUM OFFSET(6) NUMBITS(1) [],
        EXEC_DONE OFFSET(7) NUMBITS(1) []
    ],
    UmaEcts [
        SW_CS0 OFFSET(0) NUMBITS(1) [],
        SW_CS1 OFFSET(1) NUMBITS(1) [],
        SW_CS2 OFFSET(2) NUMBITS(1) [],
        DEV_NUM_BACK OFFSET(3) NUMBITS(1) [],
        UMA_ADDR_SIZE OFFSET(4) NUMBITS(3) []
    ],
    Addr4bEn [
        PVT_4B OFFSET(4) NUMBITS(1) [],
        SHD_4B OFFSET(5) NUMBITS(1) [],
        BACK_4B OFFSET(6) NUMBITS(1) []
    ],
    FiuMsrSts [
        RD_PEND_UMA OFFSET(0) NUMBITS(1) [],
        RD_PEND_FIU OFFSET(1) NUMBITS(1) [],
        MSTR_INACT OFFSET(2) NUMBITS(1) []
    ],
    FiuMsrIeCfg [
        RD_PEND_UMA_IE OFFSET(0) NUMBITS(1) [],
        RD_PEND_FIU_IE OFFSET(1) NUMBITS(1) [],
        MSTR_INACT_IE OFFSET(2) NUMBITS(1) [],
        UMA_BLOCK OFFSET(3) NUMBITS(1) []
    ],
    UmaCode [
        CODE OFFSET(0) NUMBITS(8) []
    ],
    UmaAb0 [
        ADDR_BYTE_0 OFFSET(0) NUMBITS(8) []
    ],
    UmaAb1 [
        ADDR_BYTE_1 OFFSET(0) NUMBITS(8) []
    ],
    UmaAb2 [
        ADDR_BYTE_2 OFFSET(0) NUMBITS(8) []
    ],
    UmaDb0 [
        DATA_BYTE_0 OFFSET(0) NUMBITS(8) []
    ],
    UmaDb1 [
        DATA_BYTE_1 OFFSET(0) NUMBITS(8) []
    ],
    UmaDb2 [
        DATA_BYTE_2 OFFSET(0) NUMBITS(8) []
    ],
    UmaDb3 [
        DATA_BYTE_3 OFFSET(0) NUMBITS(8) []
    ]
];

/// FIU constants
const FIU_CHK_TIMEOUT_US: u32 = 1_000;

/// Flash command opcodes
const CMD_READ_STATUS: u8 = 0x05;
const CMD_WRITE_ENABLE: u8 = 0x06;
const CMD_JEDEC_ID: u8 = 0x9F;

/// UMA control field constants
const UMA_NO_DATA: u8 = 0;
const UMA_ONE_BYTE: u8 = 1;
const UMA_NO_ADDR: u8 = 0;

/// System register addresses for power and pinmux control
const PWDWN_CTL1_ADDR: usize = 0x4000_D008;
const PWDWN_CTL1_BIT: u32 = 1 << 2;
const SHD_SPI_TRIS_ADDR: usize = 0x400C_3000;
const SHD_SPI_TRIS_BIT: u32 = 1 << 6;
const SHD_SPI_CTRL_ADDR: usize = 0x400C_301C;
const SHD_SPI_BIT: u32 = 1 << 3;
const SHD_SPI_QUAD_BIT: u32 = 1 << 2;
const WP_GPIO55_ADDR: usize = 0x400C_3004;
const WP_GPIO55_BIT: u32 = 1 << 1;

/// FIU chip select
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FiuChipSelect {
    Private,
    Shared,
    Backup,
}

impl FiuChipSelect {
    fn sw_index(&self) -> u8 {
        match self {
            FiuChipSelect::Private => 0,
            FiuChipSelect::Shared => 1,
            FiuChipSelect::Backup => 2,
        }
    }
}

/// FIU read modes
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FiuReadMode {
    Normal,
    Fast,
    FastDual,
    Quad,
}

/// FIU configuration
#[derive(Copy, Clone, Debug)]
pub struct FiuConfig {
    pub chip_select: FiuChipSelect,
    pub read_mode: FiuReadMode,
    pub enter_4ba: bool,
}

impl FiuConfig {
    const fn default() -> Self {
        Self {
            chip_select: FiuChipSelect::Private,
            read_mode: FiuReadMode::Normal,
            enter_4ba: false,
        }
    }
}

/// Flash operation state for deferred callbacks
#[derive(Copy, Clone, Debug, PartialEq)]
enum FlashState {
    Idle,
    Read,
    Write,
    Erase,
    EraseError,
}

/// FIU driver for Tock kernel
pub struct Fiu<'a> {
    registers_core: StaticRef<FiuRegisters>,
    registers_host: StaticRef<FiuRegisters>,
    config: Cell<FiuConfig>,
    client: OptionalCell<&'a dyn hil::flash::Client<Fiu<'a>>>,
    page_buffer: TakeCell<'static, FiuPage>,
    state: Cell<FlashState>,
    erase_result: Cell<Result<(), hil::flash::Error>>,
    deferred_call: DeferredCall,
}

// Forward declare FiuPage so we can use it in the struct definition
// (actual definition comes later in the file after Flash HIL section)
pub struct FiuPage(pub [u8; 256]);

impl<'a> Fiu<'a> {
    pub fn new(deferred_call: DeferredCall) -> Self {
        Self {
            registers_core: unsafe { StaticRef::new(FIU0_BASE as *const FiuRegisters) },
            registers_host: unsafe { StaticRef::new(FIU1_BASE as *const FiuRegisters) },
            config: Cell::new(FiuConfig::default()),
            client: OptionalCell::empty(),
            page_buffer: TakeCell::empty(),
            state: Cell::new(FlashState::Idle),
            erase_result: Cell::new(Ok(())),
            deferred_call,
        }
    }

    /// Initialize FIU hardware
    pub fn initialize(&self) -> Result<(), ErrorCode> {
        self.init_power_control()?;
        self.init_pinmux()?;
        Ok(())
    }

    fn init_power_control(&self) -> Result<(), ErrorCode> {
        // Optimized: use volatile pointer operations more efficiently
        unsafe {
            let reg = PWDWN_CTL1_ADDR as *mut u32;
            let val = reg.read_volatile();
            reg.write_volatile(val & !PWDWN_CTL1_BIT);
        }
        Ok(())
    }

    fn init_pinmux(&self) -> Result<(), ErrorCode> {
        // Optimized: minimize volatile operations by caching reads
        unsafe {
            // Clear SHD_SPI_TRIS bit
            let reg = SHD_SPI_TRIS_ADDR as *mut u32;
            let val = reg.read_volatile();
            reg.write_volatile(val & !SHD_SPI_TRIS_BIT);

            // Set SHD_SPI and QUAD bits
            let reg = SHD_SPI_CTRL_ADDR as *mut u32;
            let val = reg.read_volatile();
            reg.write_volatile(val | SHD_SPI_BIT | SHD_SPI_QUAD_BIT);

            // Disable write protection by clearing WP_GPIO55 bit
            let reg = WP_GPIO55_ADDR as *mut u32;
            let val_before = reg.read_volatile();
            reg.write_volatile(val_before & !WP_GPIO55_BIT);
        }
        Ok(())
    }

    pub fn configure(&self, config: &FiuConfig) -> Result<(), ErrorCode> {
        kernel::debug!(
            "[FIU] configure: chip_select={:?}, read_mode={:?}, enter_4ba={}",
            config.chip_select,
            config.read_mode,
            config.enter_4ba
        );
        self.config.set(*config);
        self.config_uma_mode(config)?;
        self.config_dra_mode(config)?;
        Ok(())
    }

    fn config_uma_mode(&self, config: &FiuConfig) -> Result<(), ErrorCode> {
        if config.chip_select == FiuChipSelect::Backup {
            self.registers_core
                .uma_ects
                .modify(UmaEcts::DEV_NUM_BACK::SET);
        } else {
            self.registers_core
                .uma_ects
                .modify(UmaEcts::DEV_NUM_BACK::CLEAR);
        }
        Ok(())
    }

    fn config_dra_mode(&self, config: &FiuConfig) -> Result<(), ErrorCode> {
        // Helper closure to configure both core and host registers
        let configure_both = |core_regs: &FiuRegisters, host_regs: &FiuRegisters| {
            // Set read mode based on configuration
            let rd_mode = match config.read_mode {
                FiuReadMode::Normal => SpiFlCfg::RD_MODE::NORMAL,
                FiuReadMode::Fast => SpiFlCfg::RD_MODE::FAST,
                FiuReadMode::FastDual | FiuReadMode::Quad => SpiFlCfg::RD_MODE::FAST_DUAL,
            };
            core_regs.spi_fl_cfg.modify(rd_mode);
            host_regs.spi_fl_cfg.modify(rd_mode);

            // Set quad mode
            let quad_cfg = if config.read_mode == FiuReadMode::Quad {
                RespCfg::QUAD_EN::SET
            } else {
                RespCfg::QUAD_EN::CLEAR
            };
            core_regs.resp_cfg.modify(quad_cfg);
            host_regs.resp_cfg.modify(quad_cfg);

            // Set burst config to 16B
            core_regs.burst_cfg.modify(BurstCfg::R_BURST::BURST_16B);
            host_regs.burst_cfg.modify(BurstCfg::R_BURST::BURST_16B);

            // Configure 4-byte address mode based on chip select
            let addr_4b_field = match config.chip_select {
                FiuChipSelect::Private => {
                    if config.enter_4ba {
                        Addr4bEn::PVT_4B::SET
                    } else {
                        Addr4bEn::PVT_4B::CLEAR
                    }
                }
                FiuChipSelect::Shared => {
                    if config.enter_4ba {
                        Addr4bEn::SHD_4B::SET
                    } else {
                        Addr4bEn::SHD_4B::CLEAR
                    }
                }
                FiuChipSelect::Backup => {
                    if config.enter_4ba {
                        Addr4bEn::BACK_4B::SET
                    } else {
                        Addr4bEn::BACK_4B::CLEAR
                    }
                }
            };
            core_regs.addr_4b_en.modify(addr_4b_field);
            host_regs.addr_4b_en.modify(addr_4b_field);
        };

        configure_both(&self.registers_core, &self.registers_host);
        Ok(())
    }

    fn uma_lock(&self) -> Result<(), ErrorCode> {
        let mut timeout = FIU_CHK_TIMEOUT_US;

        while timeout > 0 {
            if self
                .registers_host
                .fiu_msr_sts
                .is_set(FiuMsrSts::MSTR_INACT)
            {
                break;
            }
            timeout -= 1;
        }

        if timeout == 0 {
            kernel::debug!("[FIU] uma_lock: timeout waiting for host inactive");
            return Err(ErrorCode::FAIL);
        }

        self.registers_core
            .fiu_msr_ie_cfg
            .modify(FiuMsrIeCfg::UMA_BLOCK::SET);
        Ok(())
    }

    fn uma_release(&self) {
        self.registers_core
            .fiu_msr_ie_cfg
            .modify(FiuMsrIeCfg::UMA_BLOCK::CLEAR);
    }

    fn wait_uma_complete(&self) -> Result<(), ErrorCode> {
        let mut timeout = FIU_CHK_TIMEOUT_US;

        while timeout > 0 {
            if !self.registers_core.uma_cts.is_set(UmaCts::EXEC_DONE) {
                break;
            }
            timeout -= 1;
        }

        if timeout == 0 {
            kernel::debug!("[FIU] wait_uma_complete: timeout - UMA transaction did not complete");
            return Err(ErrorCode::FAIL);
        }

        Ok(())
    }

    /// Execute a UMA transaction with the specified parameters
    fn uma_execute(
        &self,
        d_size: u8,
        a_size: u8,
        no_cmd: bool,
        is_write: bool,
    ) -> Result<(), ErrorCode> {
        let config = self.config.get();

        self.registers_core.uma_cts.write(
            UmaCts::D_SIZE.val(d_size)
                + UmaCts::A_SIZE.val(a_size)
                + if no_cmd {
                    UmaCts::C_SIZE::SET
                } else {
                    UmaCts::C_SIZE::CLEAR
                }
                + if is_write {
                    UmaCts::RD_WR::SET
                } else {
                    UmaCts::RD_WR::CLEAR
                }
                + if config.chip_select == FiuChipSelect::Shared {
                    UmaCts::DEV_NUM::SET
                } else {
                    UmaCts::DEV_NUM::CLEAR
                }
                + UmaCts::EXEC_DONE::SET, // Trigger UMA transaction execution
        );

        self.wait_uma_complete()
    }

    fn uma_write_byte(&self, data: u8) -> Result<(), ErrorCode> {
        // Set command byte
        self.registers_core.uma_code.set(data);
        // Execute: write command byte only (D_SIZE=0, A_SIZE=0, has_cmd, write)
        self.uma_execute(UMA_NO_DATA, UMA_NO_ADDR, false, true)
    }

    fn uma_read_byte(&self) -> Result<u8, ErrorCode> {
        // Clear uma_code to ensure we don't accidentally send a command byte
        // The no_cmd flag in uma_execute will prevent command transmission,
        // but it's safer to explicitly clear the command register
        self.registers_core.uma_code.set(0x00);

        // Execute: read 1 byte with no command (D_SIZE=1, A_SIZE=0, no_cmd, read)
        self.uma_execute(UMA_ONE_BYTE, UMA_NO_ADDR, true, false)?;

        let data = self.registers_core.uma_db0.get();

        Ok(data)
    }

    fn set_cs_level(&self, sw_cs: u8, level: bool) {
        // Get cs field to modify
        let cs_field = match sw_cs {
            0 => Some(if level {
                UmaEcts::SW_CS0::SET
            } else {
                UmaEcts::SW_CS0::CLEAR
            }),
            1 => Some(if level {
                UmaEcts::SW_CS1::SET
            } else {
                UmaEcts::SW_CS1::CLEAR
            }),
            2 => Some(if level {
                UmaEcts::SW_CS2::SET
            } else {
                UmaEcts::SW_CS2::CLEAR
            }),
            _ => {
                kernel::debug!("[FIU] set_cs_level: invalid sw_cs={}", sw_cs);
                None
            }
        };

        if let Some(field) = cs_field {
            self.registers_core.uma_ects.modify(field);
        }
    }

    pub fn read_status(&self) -> Result<u8, ErrorCode> {
        self.uma_lock()?;

        // Assert chip select
        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send Read Status Register command
        self.uma_write_byte(CMD_READ_STATUS)?;

        let status = self.uma_read_byte()?;

        // Deassert chip select
        self.set_cs_level(sw_cs, true);

        self.uma_release();

        Ok(status)
    }

    pub fn write_enable(&self) -> Result<(), ErrorCode> {
        self.uma_lock()?;

        // Assert chip select
        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send Write Enable command
        self.uma_write_byte(CMD_WRITE_ENABLE)?;

        // Deassert chip select
        self.set_cs_level(sw_cs, true);

        self.uma_release();

        // Verify WEL bit is set (bit 1 of status register) for diagnostics
        // Don't fail here - let the actual flash operation fail if WEL is not set
        if let Ok(status) = self.read_status() {
            let wel = (status & 0x02) != 0;

            if !wel {
                kernel::debug!("[FIU] WARNING: WEL bit not set after WREN - write/erase may fail!");
            }
        }

        Ok(())
    }

    pub fn read_jedec_id(&self) -> Result<[u8; 3], ErrorCode> {
        self.uma_lock()?;

        // Assert chip select
        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send JEDEC ID command
        self.uma_write_byte(CMD_JEDEC_ID)?;

        // Read 3 bytes of JEDEC ID
        let id0 = self.uma_read_byte()?;

        let id1 = self.uma_read_byte()?;

        let id2 = self.uma_read_byte()?;

        // Deassert chip select
        self.set_cs_level(sw_cs, true);

        self.uma_release();

        Ok([id0, id1, id2])
    }
}

// Flash HIL implementation
// Page size for FIU flash (standard SPI NOR flash page size)
const FLASH_PAGE_SIZE: usize = 256;

// Implement traits for FiuPage (struct already defined earlier)
impl Default for FiuPage {
    fn default() -> Self {
        Self([0; 256])
    }
}

impl core::ops::Index<usize> for FiuPage {
    type Output = u8;

    fn index(&self, idx: usize) -> &u8 {
        &self.0[idx]
    }
}

impl core::ops::IndexMut<usize> for FiuPage {
    fn index_mut(&mut self, idx: usize) -> &mut u8 {
        &mut self.0[idx]
    }
}

impl AsMut<[u8]> for FiuPage {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

// DeferredCallClient implementation for handling callbacks outside syscall context
impl<'a> DeferredCallClient for Fiu<'a> {
    fn handle_deferred_call(&self) {
        self.handle_interrupt();
    }

    fn register(&'static self) {
        self.deferred_call.register(self);
    }
}

impl<'a> Fiu<'a> {
    fn handle_interrupt(&self) {
        match self.state.get() {
            FlashState::Read => {
                // Set state to Idle BEFORE calling callback to allow reentrancy
                self.state.set(FlashState::Idle);
                self.client.map(|client| {
                    self.page_buffer.take().map(|buffer| {
                        client.read_complete(buffer, Ok(()));
                    });
                });
            }
            FlashState::Write => {
                // Set state to Idle BEFORE calling callback to allow reentrancy
                self.state.set(FlashState::Idle);
                self.client.map(|client| {
                    self.page_buffer.take().map(|buffer| {
                        client.write_complete(buffer, Ok(()));
                    });
                });
            }
            FlashState::Erase => {
                // Set state to Idle BEFORE calling callback to allow reentrancy
                self.state.set(FlashState::Idle);
                self.client.map(|client| {
                    client.erase_complete(Ok(()));
                });
            }
            FlashState::EraseError => {
                // Set state to Idle BEFORE calling callback to allow reentrancy
                self.state.set(FlashState::Idle);
                let result = self.erase_result.get();
                self.client.map(|client| {
                    client.erase_complete(result);
                });
            }
            FlashState::Idle => {}
        }
    }
}

impl<'a, C: hil::flash::Client<Fiu<'a>>> hil::flash::HasClient<'a, C> for Fiu<'a> {
    fn set_client(&self, client: &'a C) {
        self.client.set(client);
    }
}

impl<'a> hil::flash::Flash for Fiu<'a> {
    type Page = FiuPage;

    fn read_page(
        &self,
        page_number: usize,
        buf: &'static mut Self::Page,
    ) -> Result<(), (ErrorCode, &'static mut Self::Page)> {
        // Check if busy
        if self.state.get() != FlashState::Idle {
            return Err((ErrorCode::BUSY, buf));
        }

        // Calculate flash address from page number
        let address = page_number * FLASH_PAGE_SIZE;

        // Perform synchronous read operation
        match self.read_data(address as u32, &mut buf.0) {
            Ok(_) => {
                // Save buffer for callback
                self.page_buffer.replace(buf);

                // Set state and trigger deferred call
                self.state.set(FlashState::Read);
                self.deferred_call.set();

                Ok(())
            }
            Err(e) => Err((e, buf)),
        }
    }

    fn write_page(
        &self,
        page_number: usize,
        buf: &'static mut Self::Page,
    ) -> Result<(), (ErrorCode, &'static mut Self::Page)> {
        // Check if busy
        if self.state.get() != FlashState::Idle {
            return Err((ErrorCode::BUSY, buf));
        }

        // Calculate flash address from page number
        let address = page_number * FLASH_PAGE_SIZE;

        // Perform synchronous write operation
        match self.write_data(address as u32, &buf.0) {
            Ok(_) => {
                // Save buffer for callback
                self.page_buffer.replace(buf);

                // Set state and trigger deferred call
                self.state.set(FlashState::Write);
                self.deferred_call.set();

                Ok(())
            }
            Err(e) => Err((e, buf)),
        }
    }

    fn erase_page(&self, page_number: usize) -> Result<(), ErrorCode> {
        // Check if busy
        if self.state.get() != FlashState::Idle {
            return Err(ErrorCode::BUSY);
        }

        // Flash sectors are typically 4KB (4096 bytes)
        // Since our page is 256 bytes, we need to erase the sector containing this page
        const SECTOR_SIZE: usize = 4096;

        // Calculate sector address
        let page_address = page_number * FLASH_PAGE_SIZE;
        let sector_address = (page_address / SECTOR_SIZE) * SECTOR_SIZE;

        // Perform synchronous erase operation
        let result = self.erase_sector(sector_address as u32);

        // Always set state and trigger deferred call to deliver result (success or failure)
        // This ensures async callbacks are always triggered even on error
        match result {
            Ok(_) => {
                self.state.set(FlashState::Erase);
                self.erase_result.set(Ok(()));
            }
            Err(e) => {
                self.state.set(FlashState::EraseError);
                self.erase_result.set(Err(hil::flash::Error::FlashError));
                kernel::debug!("[FIU] Erase failed: {:?}", e);
            }
        }

        self.deferred_call.set();
        Ok(())
    }
}

// Helper methods for low-level flash operations
impl<'a> Fiu<'a> {
    /// Read data from flash at specified address
    pub fn read_data(&self, address: u32, buf: &mut [u8]) -> Result<(), ErrorCode> {
        const CMD_READ: u8 = 0x03;

        self.uma_lock()?;

        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send READ command
        self.uma_write_byte(CMD_READ)?;

        // Send 24-bit address
        self.uma_write_byte((address >> 16) as u8)?;
        self.uma_write_byte((address >> 8) as u8)?;
        self.uma_write_byte(address as u8)?;

        // Wait for address transmission to complete
        self.wait_uma_complete()?;

        // Read data bytes
        for byte in buf.iter_mut() {
            *byte = self.uma_read_byte()?;
        }

        self.set_cs_level(sw_cs, true);
        self.uma_release();

        Ok(())
    }

    /// Write data to flash at specified address (page program)
    fn write_data(&self, address: u32, data: &[u8]) -> Result<(), ErrorCode> {
        const CMD_PAGE_PROGRAM: u8 = 0x02;

        // Enable write
        self.write_enable()?;

        self.uma_lock()?;

        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send PAGE PROGRAM command
        self.uma_write_byte(CMD_PAGE_PROGRAM)?;

        // Send 24-bit address
        self.uma_write_byte((address >> 16) as u8)?;
        self.uma_write_byte((address >> 8) as u8)?;
        self.uma_write_byte(address as u8)?;

        // Write data bytes
        for &byte in data.iter() {
            self.uma_write_byte(byte)?;
        }

        self.set_cs_level(sw_cs, true);
        self.uma_release();

        // Wait for write to complete
        self.wait_for_ready()?;

        Ok(())
    }

    /// Erase a 4KB sector
    fn erase_sector(&self, address: u32) -> Result<(), ErrorCode> {
        const CMD_SECTOR_ERASE: u8 = 0x20;

        // Enable write
        self.write_enable()?;

        self.uma_lock()?;

        let sw_cs = self.config.get().chip_select.sw_index();
        self.set_cs_level(sw_cs, false);

        // Send SECTOR ERASE command
        self.uma_write_byte(CMD_SECTOR_ERASE)?;

        // Send 24-bit address
        self.uma_write_byte((address >> 16) as u8)?;
        self.uma_write_byte((address >> 8) as u8)?;
        self.uma_write_byte(address as u8)?;

        // Wait for erase command transmission to complete
        self.wait_uma_complete()?;

        self.set_cs_level(sw_cs, true);
        self.uma_release();

        // Wait for erase to complete
        self.wait_for_ready()?;

        Ok(())
    }

    /// Wait for flash to be ready (WIP bit cleared)
    fn wait_for_ready(&self) -> Result<(), ErrorCode> {
        const MAX_RETRIES: u32 = 10000;

        for _ in 0..MAX_RETRIES {
            let status = self.read_status()?;
            if (status & 0x01) == 0 {
                // WIP bit cleared, flash is ready
                return Ok(());
            }
        }

        Err(ErrorCode::FAIL)
    }
}
