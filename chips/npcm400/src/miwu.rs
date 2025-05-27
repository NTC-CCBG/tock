// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Multi-input wakeup unit

use core::cell::Cell;
use kernel::platform::chip::ClockInterface;
use kernel::utilities::registers::interfaces::{ReadWriteable, Readable, Writeable};
use kernel::utilities::registers::{
    register_bitfields, register_structs, ReadOnly, ReadWrite, WriteOnly,
};
use kernel::utilities::StaticRef;

register_structs! {
    MiwuRegisters {
        // Group 1
        (0x00 => wkedg1: ReadWrite<u8, Wkedg::Register>),      // 00h Edge Detection 1
        (0x01 => wkaedg1: ReadWrite<u8, Wkaedg::Register>),    // 01h Any Edge Detection 1
        (0x0A => wkpnd1: ReadWrite<u8, Wkpn::Register>),       // 0Ah Pending 1
        (0x0C => wkpcl1: WriteOnly<u8, Wkpcl::Register>),      // 0Ch Pending Clear 1
        (0x1E => wken1: ReadWrite<u8, Wken::Register>),        // 1Eh Enable 1
        (0x1F => wkinen1: ReadWrite<u8, Wkinen::Register>),    // 1Fh Wake-Up Input Enable 1
        (0x70 => wkmod1: ReadWrite<u8, Wkmod::Register>),      // 70h Wake-Up Detection Mode 1

        // Group 2
        (0x02 => wkedg2: ReadWrite<u8, Wkedg::Register>),      // 02h Edge Detection 2
        (0x03 => wkaedg2: ReadWrite<u8, Wkaedg::Register>),    // 03h Any Edge Detection 2
        (0x0E => wkpnd2: ReadWrite<u8, Wkpn::Register>),       // 0Eh Pending 2
        (0x10 => wkpcl2: WriteOnly<u8, Wkpcl::Register>),      // 10h Pending Clear 2
        (0x20 => wken2: ReadWrite<u8, Wken::Register>),        // 20h Enable 2
        (0x21 => wkinen2: ReadWrite<u8, Wkinen::Register>),    // 21h Wake-Up Input Enable 2
        (0x71 => wkmod2: ReadWrite<u8, Wkmod::Register>),      // 71h Wake-Up Detection Mode 2

        // Group 3
        (0x04 => wkedg3: ReadWrite<u8, Wkedg::Register>),      // 04h Edge Detection 3
        (0x05 => wkaedg3: ReadWrite<u8, Wkaedg::Register>),    // 05h Any Edge Detection 3
        (0x12 => wkpnd3: ReadWrite<u8, Wkpn::Register>),       // 12h Pending 3
        (0x14 => wkpcl3: WriteOnly<u8, Wkpcl::Register>),      // 14h Pending Clear 3
        (0x22 => wken3: ReadWrite<u8, Wken::Register>),        // 22h Enable 3
        (0x23 => wkinen3: ReadWrite<u8, Wkinen::Register>),    // 23h Wake-Up Input Enable 3
        (0x72 => wkmod3: ReadWrite<u8, Wkmod::Register>),      // 72h Wake-Up Detection Mode 3

        // Group 4
        (0x06 => wkedg4: ReadWrite<u8, Wkedg::Register>),      // 06h Edge Detection 4
        (0x07 => wkaedg4: ReadWrite<u8, Wkaedg::Register>),    // 07h Any Edge Detection 4
        (0x16 => wkpnd4: ReadWrite<u8, Wkpn::Register>),       // 16h Pending 4
        (0x18 => wkpcl4: WriteOnly<u8, Wkpcl::Register>),      // 18h Pending Clear 4
        (0x24 => wken4: ReadWrite<u8, Wken::Register>),        // 24h Enable 4
        (0x25 => wkinen4: ReadWrite<u8, Wkinen::Register>),    // 25h Wake-Up Input Enable 4
        (0x73 => wkmod4: ReadWrite<u8, Wkmod::Register>),      // 73h Wake-Up Detection Mode 4

        // Group 5
        (0x08 => wkedg5: ReadWrite<u8, Wkedg::Register>),      // 08h Edge Detection 5
        (0x09 => wkaedg5: ReadWrite<u8, Wkaedg::Register>),    // 09h Any Edge Detection 5
        (0x1A => wkpnd5: ReadWrite<u8, Wkpn::Register>),       // 1Ah Pending 5
        (0x1C => wkpcl5: WriteOnly<u8, Wkpcl::Register>),      // 1Ch Pending Clear 5
        (0x26 => wken5: ReadWrite<u8, Wken::Register>),        // 26h Enable 5
        (0x27 => wkinen5: ReadWrite<u8, Wkinen::Register>),    // 27h Wake-Up Input Enable 5
        (0x74 => wkmod5: ReadWrite<u8, Wkmod::Register>),      // 74h Wake-Up Detection Mode 5

        // Group 6
        (0x28 => wkedg6: ReadWrite<u8, Wkedg::Register>),      // 28h Edge Detection 6
        (0x29 => wkaedg6: ReadWrite<u8, Wkaedg::Register>),    // 29h Any Edge Detection 6
        (0x2E => wkpnd6: ReadWrite<u8, Wkpn::Register>),       // 2Eh Pending 6
        (0x30 => wkpcl6: WriteOnly<u8, Wkpcl::Register>),      // 30h Pending Clear 6
        (0x3A => wken6: ReadWrite<u8, Wken::Register>),        // 3Ah Enable 6
        (0x3B => wkinen6: ReadWrite<u8, Wkinen::Register>),    // 3Bh Wake-Up Input Enable 6
        (0x75 => wkmod6: ReadWrite<u8, Wkmod::Register>),      // 75h Wake-Up Detection Mode 6

        // Group 7
        (0x2A => wkedg7: ReadWrite<u8, Wkedg::Register>),      // 2Ah Edge Detection 7
        (0x2B => wkaedg7: ReadWrite<u8, Wkaedg::Register>),    // 2Bh Any Edge Detection 7
        (0x32 => wkpnd7: ReadWrite<u8, Wkpn::Register>),       // 32h Pending 7
        (0x34 => wkpcl7: WriteOnly<u8, Wkpcl::Register>),      // 34h Pending Clear 7
        (0x3C => wken7: ReadWrite<u8, Wken::Register>),        // 3Ch Enable 7
        (0x3D => wkinen7: ReadWrite<u8, Wkinen::Register>),    // 3Dh Wake-Up Input Enable 7
        (0x76 => wkmod7: ReadWrite<u8, Wkmod::Register>),      // 76h Wake-Up Detection Mode 7

        // Group 8
        (0x2C => wkedg8: ReadWrite<u8, Wkedg::Register>),      // 2Ch Edge Detection 8
        (0x2D => wkaedg8: ReadWrite<u8, Wkaedg::Register>),    // 2Dh Any Edge Detection 8
        (0x36 => wkpnd8: ReadWrite<u8, Wkpn::Register>),       // 36h Pending 8
        (0x38 => wkpcl8: WriteOnly<u8, Wkpcl::Register>),      // 38h Pending Clear 8
        (0x3E => wken8: ReadWrite<u8, Wken::Register>),        // 3Eh Enable 8
        (0x3F => wkinen8: ReadWrite<u8, Wkinen::Register>),    // 3Fh Wake-Up Input Enable 8
        (0x77 => wkmod8: ReadWrite<u8, Wkmod::Register>),      // 77h Wake-Up Detection Mode 8

        // Miscellaneous
        (0x0B => low_pnd_wu: ReadOnly<u8, Low_Pnd_wu::Register>),    // 0Bh Lowest Pending Wake-Up Byte
        (0x78 => @END),
    }
}

register_bitfields![u8,
    Wkedg [
        EDGE OFFSET(0) NUMBITS(8) [], // Each bit represents an edge sense
    ],
    Wkaedg [
        EDGE OFFSET(0) NUMBITS(8) [], // Each bit represents alternate edge sense
    ],
    Wkpn [
        PENDING OFFSET(0) NUMBITS(8) [], // Pending bits
    ],
    Wkpcl [
        CLEAR OFFSET(0) NUMBITS(8) [], // Pending clear bits
    ],
    Wken [
        ENABLE OFFSET(0) NUMBITS(8) [], // Enable bits
    ],
    Wkinen [
        INPUT_ENABLE OFFSET(0) NUMBITS(8) [], // Input enable bits
    ],
    Wkmod [
        MODE OFFSET(0) NUMBITS(8) [], // Mode: edge/level bits
    ],
    Low_Pnd_wu [
        PND_IN OFFSET(0) NUMBITS(3) [], // The lowest number of the input group of the pending wake-up
        PND_GRP OFFSET(4) NUMBITS(4) [] // The lowest input number in the PND_GRP group of the pending wake-up
    ]
];

const MIWU0_BASE: StaticRef<MiwuRegisters> =
    unsafe { StaticRef::new(0x400B_B000 as *const MiwuRegisters) };
const MIWU1_BASE: StaticRef<MiwuRegisters> =
    unsafe { StaticRef::new(0x400B_D000 as *const MiwuRegisters) };
const MIWU2_BASE: StaticRef<MiwuRegisters> =
    unsafe { StaticRef::new(0x400B_F000 as *const MiwuRegisters) };

pub struct Miwu {
    registers: StaticRef<MiwuRegisters>,
    enabled: Cell<bool>,
}

impl Miwu {
    fn new(reg: StaticRef<MiwuRegisters>) -> Self {
        Self {
            registers: reg,
            enabled: Cell::new(false),
        }
    }

    pub fn new_miwu0(&self) -> Self {
        Self::new(MIWU0_BASE)
    }

    pub fn new_miwu1(&self) -> Self {
        Self::new(MIWU1_BASE)
    }

    pub fn new_miwu2(&self) -> Self {
        Self::new(MIWU2_BASE)
    }

    // pub fn intc_miwu_isr(miwu: &Miwu, data: u32) {
    //     let table = npcm_miwu_isr_table_offset(data);
    //     let mut group_mask = npcm_miwu_isr_group_mask_offset(data);
    //     let mut group = 0u8;

    //     // Check all MIWU groups belonging to the same IRQ
    //     while group_mask != 0 {
    //         if (group_mask & 0x01) != 0 {
    //             let wk_src_idx = npcm_miwu_table_offset(table) + npcm_miwu_group_offset(group);
    //             let wui = NpcmWui { wk_src_idx };

    //             npcm_miwu_isr_priority(miwu, &wui);
    //         }
    //         group += 1;
    //         group_mask >>= 1;
    //     }
    // }

    // fn npcm_miwu_isr_priority(miwu: &Miwu, wui: &NpcmWui) {
    //     // Get group from wk_src_idx
    //     let group = npcm_wui_group_offset(wui.wk_src_idx);

    //     // Get callback list for this group (assume you have a way to get this)
    //     let cb_list = miwu.get_callback_list(group);

    //     // Read pending and enable bits
    //     let mask = miwu.read_wkpnd(group) & miwu.read_wken(group);

    //     // Clear pending bits before dispatching ISR
    //     if mask != 0 {
    //         miwu.write_wkpcl(group, mask);
    //     }

    //     // Dispatch registered GPIO/device ISRs
    //     Miwu::interrupt_handle(cb_list, mask);
    // }
}
