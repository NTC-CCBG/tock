// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Universal asynchronous receiver/transmitter with EasyDMA (UARTE)
//!
//! Author
//! -------------------
//!
//! * Author: Niklas Adolfsson <niklasadolfsson1@gmail.com>
//! * Date: March 10 2018

use core::cell::Cell;
use core::cmp::min;
use kernel::hil::uart;
use kernel::utilities::cells::OptionalCell;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{register_bitfields, ReadOnly, ReadWrite};
use kernel::utilities::StaticRef;
use kernel::ErrorCode;

static mut BYTE: u8 = 0;

pub const UART1_BASE: StaticRef<UarteRegisters> =
    unsafe { StaticRef::new(0x400C_4000 as *const UarteRegisters) };

#[repr(C)]
pub struct UarteRegisters {
    /// 0x000: Transmit Data Buffer
    pub utbuf: ReadWrite<u8, Utbuf::Register>,
    _reserved1: [u8; 1],
    /// 0x002: Receive Data Buffer
    pub urbuf: ReadOnly<u8, Urbuf::Register>,
    _reserved2: [u8; 1],
    /// 0x004: Interrupt Control
    pub uictrl: ReadWrite<u8, Uictrl::Register>,
    _reserved3: [u8; 1],
    /// 0x006: Status
    pub ustat: ReadOnly<u8, Ustat::Register>,
    _reserved4: [u8; 1],
    /// 0x008: Frame Select
    pub ufrs: ReadWrite<u8, Ufrs::Register>,
    _reserved5: [u8; 1],
    /// 0x00A: Mode Select
    pub umdsl: ReadWrite<u8, Umdsl::Register>,
    _reserved6: [u8; 1],
    /// 0x00C: Baud Rate Divisor
    pub ubaud: ReadWrite<u8, Ubaud::Register>,
    _reserved7: [u8; 1],
    /// 0x00E: Baud Rate Prescaler
    pub upsr: ReadWrite<u8, Upsr::Register>,
    _reserved8: [u8; 7],
    /// 0x016: FIFO Control
    pub ufctrl: ReadWrite<u8, Ufctrl::Register>,
    _reserved9: [u8; 1],
    /// 0x018: TX FIFO Current Level
    pub utxflv: ReadOnly<u8, Utxflv::Register>,
    _reserved10: [u8; 1],
    /// 0x01A: RX FIFO Current Level
    pub urxflv: ReadOnly<u8, Urxflv::Register>,
    _reserved11: [u8; 1],
}

register_bitfields! [u8,
    Utbuf [
        UTBUF OFFSET(0) NUMBITS(8),
    ],
    Urbuf [
        URBUF OFFSET(0) NUMBITS(8),
    ],
    Uictrl [
        TBE OFFSET(0) NUMBITS(1),
        RBF OFFSET(1) NUMBITS(1),
        ETI OFFSET(5) NUMBITS(1),
        ERI OFFSET(6) NUMBITS(1),
        EEI OFFSET(7) NUMBITS(1)
    ],
    Ustat [
        PE OFFSET(0) NUMBITS(1),
        FE OFFSET(1) NUMBITS(1),
        DOE OFFSET(2) NUMBITS(1),
        ERR OFFSET(3) NUMBITS(1),
        BKD OFFSET(4) NUMBITS(1),
        RB9 OFFSET(5) NUMBITS(1),
        XMIP OFFSET(6) NUMBITS(1)
    ],
    Ufrs [
        CHAR OFFSET(0) NUMBITS(2),
        STP OFFSET(2) NUMBITS(1),
        XB9 OFFSET(3) NUMBITS(1),
        PSEL OFFSET(4) NUMBITS(1),
        PEN OFFSET(6) NUMBITS(1)
    ],
    Umdsl [
        ATN OFFSET(1) NUMBITS(1),
        BRK OFFSET(2) NUMBITS(1)
    ],
    Ubaud [
        UDIV7_0 OFFSET(0) NUMBITS(7)
    ],
    Upsr [
        UDIV10_8 OFFSET(0) NUMBITS(3),
        UPSC OFFSET(3) NUMBITS(5)
    ],
    Ufctrl [
        FIFO_EN OFFSET(0) NUMBITS(1),
        EXT_LOOPBACK OFFSET(1) NUMBITS(1),
        RXFTH  OFFSET(6) NUMBITS(2)
    ],
    Utxflv [
        TFL OFFSET(0) NUMBITS(5)
    ],
    Urxflv [
        RFL OFFSET(0) NUMBITS(5)
    ]
];

const TX_FIFO_SIZE: u8 = 16;

/// UART1
// It should never be instanced outside this module but because a static mutable reference to it
// is exported outside this module it must be `pub`
pub struct Uart<'a> {
    registers: StaticRef<UarteRegisters>,
    tx_client: OptionalCell<&'a dyn uart::TransmitClient>,
    tx_buffer: kernel::utilities::cells::TakeCell<'static, [u8]>,
    tx_len: Cell<usize>,
    rx_client: OptionalCell<&'a dyn uart::ReceiveClient>,
    rx_buffer: kernel::utilities::cells::TakeCell<'static, [u8]>,
    rx_len: Cell<usize>,
    src_freq: u32,
}

#[derive(Copy, Clone)]
pub struct UARTParams {
    pub baud_rate: u32,
}

impl<'a> Uart<'a> {
    /// Constructor
    // This should only be constructed once
    fn new(regs: StaticRef<UarteRegisters>, src: u32) -> Uart<'a> {
        Uart {
            registers: regs,
            tx_client: OptionalCell::empty(),
            tx_buffer: kernel::utilities::cells::TakeCell::empty(),
            tx_len: Cell::new(0),
            rx_client: OptionalCell::empty(),
            rx_buffer: kernel::utilities::cells::TakeCell::empty(),
            rx_len: Cell::new(0),
            src_freq: src,
        }
    }

    pub fn new_uart1(src: u32) -> Self {
        Self::new(UART1_BASE, src)
    }

    /// Configure which pins the UART should use for txd, rxd, cts and rts
    pub fn initialize(&self, baud_rate: u32) {
        self.set_baud_rate(baud_rate, self.src_freq);
    }

    fn set_baud_rate_prescaler(&self, baud_rate: u32, src_freq: u32) {
        let mut opt_prescalar = 0u8;
        let mut opt_dev = 0u16;
        let mut prescalar = 10u32;
        let mut min_deviation = u32::MAX;
        let clk = src_freq;

        for i in 1..=31 {
            let mut div = (clk * 10) / (16 * baud_rate * prescalar);
            if div == 0 {
                div = 1;
            }

            let calc_baudrate = (clk * 10) / (16 * div * prescalar);
            let deviation = if calc_baudrate > baud_rate {
                calc_baudrate - baud_rate
            } else {
                baud_rate - calc_baudrate
            };

            if deviation < min_deviation {
                min_deviation = deviation;
                opt_prescalar = i as u8;
                opt_dev = div as u16;
            }
            prescalar += 5;
        }

        opt_dev = opt_dev.saturating_sub(1);

        let upsr_val: u8 = ((opt_dev >> 8) as u8 & 0x07) | ((opt_prescalar << 3) & 0xF8);
        let ubaud_val: u8 = opt_dev as u8;

        // Write to registers
        self.registers.upsr.set(upsr_val);
        self.registers.ubaud.set(ubaud_val);
    }

    fn fifo_enable(&self) {
        // Set UFRS to 0x00 for the new divisor to take effect.
        self.registers.ufrs.set(0x00);

        // If using interrupt-driven UART, enable FIFO and configure interrupts.
        #[cfg(feature = "uart_interrupt_driven")]
        {
            // Enable FIFO
            self.registers.ufctrl.modify(Ufctrl::FIFO_EN::SET);

            // Disable all UART tx FIFO interrupts
            self.irq_rx_disable();
            self.irq_tx_disable();

            // Clear UART rx FIFO
            self.clear_rx_fifo();

            // Configure UART interrupts            
            self.irq_err_enable();
        }
    }

    pub fn set_baud_rate(&self, baud_rate: u32, src_freq: u32) {
        self.set_baud_rate_prescaler(baud_rate, src_freq);
        // 8-N-1, FIFO enabled. Must be done after setting the divisor
        // for the new divisor to take effect.
        self.fifo_enable();
    }

    #[allow(dead_code)]
    fn err_check(&self) -> u32 {
        // Read the status register
        let stat = self.registers.ustat.get();
        let mut err: u32 = 0;

        // Constants for error bits (define as needed)
        const UART_ERROR_OVERRUN: u32 = 0x01;
        const UART_ERROR_PARITY: u32 = 0x02;
        const UART_ERROR_FRAMING: u32 = 0x04;

        // Check for overrun error
        if (stat & (1 << 2)) != 0 {
            err |= UART_ERROR_OVERRUN;
        }
        // Check for parity error
        if (stat & (1 << 0)) != 0 {
            err |= UART_ERROR_PARITY;
        }
        // Check for framing error
        if (stat & (1 << 1)) != 0 {
            err |= UART_ERROR_FRAMING;
        }

        err
    }

    /// FIFO rx clear
    fn clear_rx_fifo(&self) {
        // Clear all bytes from the RX FIFO by reading until empty.
        // Read all dummy bytes out from Rx FIFO
        while self.rx_fifo_ready() {
            let _ = self.registers.urbuf.get();
        }
    }

    /// FIFO ready
    fn rx_fifo_ready(&self) -> bool {
        // Returns true if RX FIFO has data available.
        self.registers.urxflv.read(Urxflv::RFL) != 0
    }

    fn rx_fifo_available(&self) -> u8 {
        // Returns the number of bytes available in the RX FIFO, capped at 16.
        self.registers.urxflv.read(Urxflv::RFL).min(TX_FIFO_SIZE)
    }

    fn tx_fifo_ready(&self) -> bool {
        // True if the Tx FIFO contains some space available
        // NPCM_UTXFLV_TFL is bits 0..=4 (5 bits), FIFO is full at 16
        self.registers.utxflv.read(Utxflv::TFL) < TX_FIFO_SIZE
    }

    fn tx_fifo_available(&self) -> u8 {
        // Returns the number of bytes available in the Tx FIFO.
        TX_FIFO_SIZE.saturating_sub(self.registers.utxflv.read(Utxflv::TFL))
    }

    /// IRQ Err
    #[allow(dead_code)]
    fn irq_err_enable(&self) {
        // Set the EEI (Enable Error Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::EEI::SET);
    }

    #[allow(dead_code)]
    fn irq_err_disable(&self) {
        // Clear the EEI (Enable Error Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::EEI::CLEAR);
    }

    /// IRQ Pending
    #[allow(dead_code)]
    fn irq_is_pending(&self) -> bool {
        // Returns true if either TX or RX interrupt is pending.
        self.tx_fifo_ready() || self.rx_fifo_ready()
    }

    /// IRQ tx enable/disable
    fn irq_tx_disable(&self) {
        // Clear the ETI (Enable Transmit Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::ETI::CLEAR);
    }

    fn irq_tx_enable(&self) {
        // Set the ETI (Enable Transmit Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::ETI::SET);
    }

    /// IRQ tx complete
    pub fn irq_tx_complete(&self) -> bool {
        // Returns true if the Tx FIFO is empty or the last byte is sending.
        // Equivalent to: !IS_BIT_SET(inst->USTAT, NPCM_USTAT_XMIP)
        self.registers.ustat.read(Ustat::XMIP) == 0
    }

    /// IRQ rx enable/disable
    fn irq_rx_disable(&self) {
        // Clear the ERI (Enable Receive Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::ERI::CLEAR);
    }

    fn irq_rx_enable(&self) {
        // Set the ERI (Enable Receive Interrupt) bit in UICtrl register.
        self.registers.uictrl.modify(Uictrl::ERI::SET);
    }

    /// FIFO
    fn fifo_fill(&self, tx_data: &[u8], size: usize) -> bool {
        let capped_size = min(size, tx_data.len());
        if capped_size == 0 {
            return false;
        }

        let mut bytes_sent = 0;
        while bytes_sent < capped_size {
            // Send one byte at a time, adding \r before \n
            let byte = tx_data[bytes_sent];

            // Add carriage return before newline for proper terminal output alignment
            if byte == b'\n' {
                // Wait for space if needed, then send carriage return
                while self.tx_fifo_available() == 0 {
                    // Wait for FIFO space
                }
                self.registers.utbuf.set(b'\r');
            }

            // Wait for space if needed, then send the actual byte
            while self.tx_fifo_available() == 0 {
                // Wait for FIFO space
            }
            self.registers.utbuf.set(byte);
            bytes_sent += 1;
        }

        true
    }

    fn fifo_read(&self, rx_data: &mut [u8], size: usize) -> bool {
        let capped_size = min(size, rx_data.len());
        if capped_size == 0 {
            return false;
        }

        let mut rx_bytes = 0;
        while rx_bytes < capped_size {
            let rx_available = self.rx_fifo_available() as usize;
            if rx_available == 0 {
                // Wait until at least one byte is available
                continue;
            }

            let to_read = min(rx_available, capped_size - rx_bytes);
            for i in 0..to_read {
                rx_data[rx_bytes + i] = self.registers.urbuf.get();
            }
            rx_bytes += to_read;
        }

        true
    }

    /// UART interrupt handler that listens for both tx_end and rx_end events
    #[inline(never)]
    #[cfg(feature = "uart_interrupt_driven")]
    pub fn handle_interrupt(&self) {
        if self.tx_fifo_ready() {
            self.irq_tx_disable();

            // All bytes have been transmitted
            self.tx_client.map(|client| {
                if let Some(tx_buffer) = self.tx_buffer.take() {
                    client.transmitted_buffer(tx_buffer, self.tx_len.get(), Ok(()));
                }
            });
        }

        if self.rx_fifo_ready() {
            self.irq_rx_disable();

            // Read data from FIFO into buffer
            if let Some(mut rx_buffer) = self.rx_buffer.take() {
                let rx_len = self.rx_len.get();
                let rx_available = self.rx_fifo_available() as usize;

                // Read up to the requested length or available bytes
                let bytes_to_read = rx_len.min(rx_available).min(rx_buffer.len());

                for i in 0..bytes_to_read {
                    rx_buffer[i] = self.registers.urbuf.get();
                }

                // Signal client that the read is done
                self.rx_client.map(|client| {
                    client.received_buffer(rx_buffer, bytes_to_read, Ok(()), uart::Error::None);
                });
            }
        }
    }

    /// Transmit one byte at the time and the client is responsible for polling
    /// This is used by the panic handler
    pub unsafe fn send_byte(&self, byte: u8) {
        // precaution: copy value into variable with static lifetime
        BYTE = byte;

        // Wait until the transmit buffer is empty (TBE bit is set)
        while self.registers.uictrl.read(Uictrl::TBE) == 0 {
            continue;
        }
        self.registers.utbuf.set(byte);
    }
}

impl<'a> uart::Transmit<'a> for Uart<'a> {
    fn set_transmit_client(&self, client: &'a dyn uart::TransmitClient) {
        self.tx_client.set(client);
    }

    fn transmit_buffer(
        &self,
        tx_data: &'static mut [u8],
        tx_len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        self.irq_tx_enable();

        if tx_data.len() < tx_len {
            return Err((ErrorCode::SIZE, tx_data));
        }

        if self.fifo_fill(tx_data, tx_len) {
            self.tx_buffer.replace(tx_data);
            self.tx_len.set(tx_len);
            Ok(())
        } else {
            Err((ErrorCode::SIZE, tx_data))
        }
    }

    fn transmit_word(&self, _data: u32) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }

    fn transmit_abort(&self) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }
}

impl uart::Configure for Uart<'_> {
    fn configure(&self, params: uart::Parameters) -> Result<(), ErrorCode> {
        // These could probably be implemented, but are currently ignored, so
        // throw an error.
        if params.stop_bits != uart::StopBits::One {
            return Err(ErrorCode::NOSUPPORT);
        }
        if params.parity != uart::Parity::None {
            return Err(ErrorCode::NOSUPPORT);
        }
        if params.hw_flow_control {
            return Err(ErrorCode::NOSUPPORT);
        }

        self.set_baud_rate(params.baud_rate, 96_000_000);

        Ok(())
    }
}

impl<'a> uart::Receive<'a> for Uart<'a> {
    fn set_receive_client(&self, client: &'a dyn uart::ReceiveClient) {
        self.rx_client.set(client);
    }

    fn receive_buffer(
        &self,
        rx_buf: &'static mut [u8],
        rx_len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        #[cfg(feature = "uart_interrupt_driven")]
        {
            // Interrupt-driven mode: non-blocking
            // Store buffer and let interrupt handler read when data arrives
            if rx_len == 0 || rx_len > rx_buf.len() {
                return Err((ErrorCode::SIZE, rx_buf));
            }

            self.rx_buffer.replace(rx_buf);
            self.rx_len.set(rx_len);
            self.irq_rx_enable();
            Ok(())
        }

        #[cfg(not(feature = "uart_interrupt_driven"))]
        {
            // Polling mode: blocking read
            self.irq_rx_enable();

            if self.fifo_read(rx_buf, rx_len) {
                self.rx_buffer.replace(rx_buf);
                self.rx_len.set(rx_len);
                Ok(())
            } else {
                Err((ErrorCode::SIZE, rx_buf))
            }
        }
    }

    fn receive_word(&self) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }

    fn receive_abort(&self) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }
}

#[cfg(test)]
mod tests {
    use kernel::ErrorCode;

    #[test]
    fn baud_rate_divider_calculation() {
        let u = super::Uart::new(super::UART1_BASE, 96_000_000);
        assert_eq!(u.get_divider_for_baud(0), Err(ErrorCode::INVAL));
        assert_eq!(u.get_divider_for_baud(4_000_000), Err(ErrorCode::INVAL));

        assert_eq!(u.get_divider_for_baud(1200), Ok(0x0004F000));
    }
}
