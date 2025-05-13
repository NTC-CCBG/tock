/* Licensed under the Apache License, Version 2.0 or the MIT License. */
/* SPDX-License-Identifier: Apache-2.0 OR MIT                         */
/* Copyright Tock Contributors 2023.                                  */

/* Memory layout for the NPCM400F
 * sram = 768KB (LENGTH = 0x00200000) */

MEMORY
{
  rom (rx)  : ORIGIN = 0x10008000, LENGTH = 256K
  ram (rwx) : ORIGIN = 0x10048000, LENGTH = 256K
  prog (rx) : ORIGIN = 0x10088000, LENGTH = 128K
}

PAGE_SIZE = 2K;
