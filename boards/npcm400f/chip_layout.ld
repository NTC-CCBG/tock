/* Licensed under the Apache License, Version 2.0 or the MIT License. */
/* SPDX-License-Identifier: Apache-2.0 OR MIT                         */
/* Copyright Tock Contributors 2023.                                  */

/* Memory layout for the NPCM400F
 * rom = 256KB (LENGTH = 0x00200000)
 * ram = 764KB */

MEMORY
{
  rom (rx)  : ORIGIN = 0x00080000, LENGTH = 256K
  prog (rx) : ORIGIN = 0x00180000, LENGTH = 100K
  ram (rwx) : ORIGIN = 0x10008000, LENGTH = 764K
}

PAGE_SIZE = 2K;
