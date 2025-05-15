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

/// UART1
// It should never be instanced outside this module but because a static mutable reference to it
// is exported outside this module it must be `pub`
pub struct Uart1<'a> {
    registers: StaticRef<UarteRegisters>,
    tx_client: OptionalCell<&'a dyn uart::TransmitClient>,
    tx_buffer: kernel::utilities::cells::TakeCell<'static, [u8]>,
    tx_len: Cell<usize>,
    tx_remaining_bytes: Cell<usize>,
    rx_client: OptionalCell<&'a dyn uart::ReceiveClient>,
    rx_buffer: kernel::utilities::cells::TakeCell<'static, [u8]>,
    rx_len: Cell<usize>,
    rx_remaining_bytes: Cell<usize>,
    // rx_abort_in_progress: Cell<bool>,
    // offset: Cell<usize>,
    // pub src_freq: u32,
}

#[derive(Copy, Clone)]
pub struct UARTParams {
    pub baud_rate: u32,
}

impl<'a> Uart1<'a> {
    /// Constructor
    // This should only be constructed once
    pub const fn new(regs: StaticRef<UarteRegisters>) -> Uart1<'a> {
        Uart1 {
            registers: regs,
            tx_client: OptionalCell::empty(),
            tx_buffer: kernel::utilities::cells::TakeCell::empty(),
            tx_len: Cell::new(0),
            tx_remaining_bytes: Cell::new(0),
            rx_client: OptionalCell::empty(),
            rx_buffer: kernel::utilities::cells::TakeCell::empty(),
            rx_len: Cell::new(0),
            rx_remaining_bytes: Cell::new(0),
            // rx_abort_in_progress: Cell::new(false),
            // offset: Cell::new(0),
            // src_freq: src,
        }
    }

    /// Configure which pins the UART should use for txd, rxd, cts and rts
    pub fn initialize(&self, src_freq: u32) {
        self.set_baud_rate(115200, src_freq);
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

        // Write to registers
        self.registers.upsr.write(
            Upsr::UDIV10_8.val(((opt_dev >> 8) & 0x7) as u8)
                + Upsr::UPSC.val((opt_prescalar << 3) & 0xF8),
        );
        self.registers
            .ubaud
            .write(Ubaud::UDIV7_0.val((opt_dev & 0xFF) as u8));
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
            self.irq_rx_enable();
            self.irq_tx_enable();

            // TODO: Debug
            unsafe {
                // devalta
                *(0x400C_301A_i32 as *mut u8) = 0x80_u8;
                // devaltc
                *(0x400C_301C_i32 as *mut u8) = 0x40_u8;
                // clock: UART_PD on
                *(0x4000_D008_i32 as *mut u8) = 0x10_u8;
            }
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
        while self.rx_fifo_available() {
            let _ = self.registers.urbuf.get();
        }
    }

    /// FIFO ready
    fn rx_fifo_available(&self) -> bool {
        // Returns true if RX FIFO has data available.
        self.registers.urxflv.read(Urxflv::RFL) != 0
    }

    fn tx_fifo_ready(&self) -> bool {
        // True if the Tx FIFO contains some space available
        // NPCM_UTXFLV_TFL is bits 0..=4 (5 bits), FIFO is full at 16
        self.registers.utxflv.read(Utxflv::TFL) < 16
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
        self.tx_fifo_ready() || self.rx_fifo_available()
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
        if size == 0 {
            return false;
        }
        let capped_size = min(size, tx_data.len());
        let mut tx_bytes = 0;

        // If Tx FIFO is still ready to send
        while (capped_size > tx_bytes) && self.tx_fifo_ready() {
            // Put a character into Tx FIFO
            self.registers.utbuf.set(tx_data[tx_bytes]);
            tx_bytes += 1;
        }

        tx_bytes > 0
    }

    fn fifo_read(&self, rx_data: &mut [u8], size: usize) -> bool {
        let mut rx_bytes = 0;

        // While at least one byte is in the Rx FIFO
        while (size - rx_bytes > 0) && self.rx_fifo_available() {
            // Receive one byte from Rx FIFO
            rx_data[rx_bytes] = self.registers.urbuf.get();
            rx_bytes += 1;
        }

        rx_bytes > 0
    }

    /// POLL
    fn poll_out(&self, c: u8) {
        while !self.fifo_fill(&[c], 1) {
            // continue looping until the byte is written
        }
    }

    fn poll_in(&self, c: &mut u8) -> i32 {
        if self.fifo_read(core::slice::from_mut(c), 1) {
            0
        } else {
            -1
        }
    }

    /// UART interrupt handler that listens for both tx_end and rx_end events
    #[inline(never)]
    #[cfg(feature = "uart_interrupt_driven")]
    pub fn handle_interrupt(&self) {
        // if self.tx_fifo_ready() {
        //     self.irq_tx_disable();

        //     let rem = self.tx_remaining_bytes.get();

        //     if rem > 0 {
        //         self.tx_buffer.map(|buf| {
        //             self.poll_out(buf[self.tx_len.get() - rem]);
        //         });
        //         self.tx_remaining_bytes.set(rem - 1);

        //         if rem - 1 == 0 {
        //             // All bytes have been transmitted
        //             self.tx_client.map(|client| {
        //                 self.tx_buffer.take().map(|tx_buffer| {
        //                     client.transmitted_buffer(tx_buffer, self.tx_len.get(), Ok(()));
        //                 });
        //             });
        //         } else {
        //             // Continue transmitting
        //             self.irq_tx_enable();
        //         }
        //     }
        // }

        // if self.rx_fifo_available() {
        //     self.irq_rx_disable();

        //     let rem = self.rx_remaining_bytes.get();

        //     if rem > 0 {
        //         self.rx_buffer.map(|buf| {
        //             self.poll_in(&mut buf[self.rx_len.get() - rem]);
        //         });
        //     }

        //     self.rx_remaining_bytes.set(rem - 1);

        //     if rem - 1 == 0 {
        //         // Signal client that the read is done
        //         self.rx_client.map(|client| {
        //             self.rx_buffer.take().map(|rx_buffer| {
        //                 client.received_buffer(
        //                     rx_buffer,
        //                     self.rx_len.get(),
        //                     Ok(()),
        //                     uart::Error::None,
        //                 );
        //             });
        //         });
        //     } else {
        //         self.irq_rx_enable();
        //     }
        // }
    }

    /// Transmit one byte at the time and the client is responsible for polling
    /// This is used by the panic handler
    pub unsafe fn send_byte(&self, byte: u8) {
        self.tx_remaining_bytes.set(1);
        // precaution: copy value into variable with static lifetime
        BYTE = byte;

        // Wait until the transmit buffer is empty (TBE bit is set)
        while self.registers.uictrl.read(Uictrl::TBE) == 0 {
            continue;
        }
        self.registers.utbuf.set(byte);
    }
}

impl<'a> uart::Transmit<'a> for Uart1<'a> {
    fn set_transmit_client(&self, client: &'a dyn uart::TransmitClient) {
        self.tx_client.set(client);
    }

    fn transmit_buffer(
        &self,
        tx_data: &'static mut [u8],
        tx_len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        if tx_len == 0 || tx_len > tx_data.len() {
            Err((ErrorCode::SIZE, tx_data))
        } else if self.tx_buffer.is_some() {
            Err((ErrorCode::BUSY, tx_data))
        } else {
            {
                // let first_byte = tx_data[0];
                // self.tx_buffer.replace(tx_data);
                // self.tx_len.set(tx_len);
                // self.tx_remaining_bytes.set(tx_len - 1);

                // self.poll_out(first_byte);

                // if tx_len > 1 {
                //     self.irq_tx_enable();
                // }
            }

            for i in 0..tx_len {
                self.poll_out(tx_data[i]);
            }

            Ok(())
        }
    }

    fn transmit_word(&self, _data: u32) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }

    fn transmit_abort(&self) -> Result<(), ErrorCode> {
        Err(ErrorCode::FAIL)
    }
}

impl uart::Configure for Uart1<'_> {
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

impl<'a> uart::Receive<'a> for Uart1<'a> {
    fn set_receive_client(&self, client: &'a dyn uart::ReceiveClient) {
        self.rx_client.set(client);
    }

    fn receive_buffer(
        &self,
        rx_buf: &'static mut [u8],
        rx_len: usize,
    ) -> Result<(), (ErrorCode, &'static mut [u8])> {
        if self.rx_buffer.is_some() {
            return Err((ErrorCode::BUSY, rx_buf));
        }

        // Determine the actual length to read
        let read_length = min(rx_len, rx_buf.len());

        // Attempt to read the first byte if the buffer is not empty
        if read_length > 0 {
            self.poll_in(&mut rx_buf[0]);

            // Store the buffer and set the remaining bytes
            self.rx_buffer.replace(rx_buf);
            self.rx_len.set(read_length);
            self.rx_remaining_bytes.set(read_length - 1);

            // Enable RX interrupt if more than one byte is expected
            if read_length > 1 {
                self.irq_rx_enable();
            }
        }

        Ok(())
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
        let u = super::Uart1::new(super::UART1_BASE);
        assert_eq!(u.get_divider_for_baud(0), Err(ErrorCode::INVAL));
        assert_eq!(u.get_divider_for_baud(4_000_000), Err(ErrorCode::INVAL));

        assert_eq!(u.get_divider_for_baud(1200), Ok(0x0004F000));
    }
}
