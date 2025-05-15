# Licensed under the Apache License, Version 2.0 or the MIT License.
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Copyright Tock Contributors 2022.

# Makefile for building the tock kernel for the NPCM400 platform
#

include ../Makefile.common

OPENOCD=openocd
POST_BUILD_SCRIPT=$(TOCK_ROOT_DIRECTORY)boards/npcm400f/esiost/esiost.py

# Default target for installing the kernel.
.PHONY: install
install: flash

.PHONY: flash-debug
flash-debug: $(TARGET_PATH)/debug/$(PLATFORM).elf
	$(OPENOCD) -c "source [find board/npcm400f.cfg]; init; reset halt; flash write_image erase $<; verify_image $<; reset; shutdown"

.PHONY: flash
flash: $(TARGET_PATH)/release/$(PLATFORM).elf
	$(OPENOCD) -c "source [find board/npcm400f.cfg]; init; reset halt; flash write_image erase $<; verify_image $<; reset; shutdown"

.PHONY: program
program: $(TARGET_PATH)/release/$(PLATFORM).elf
	$(error See README.md and update this section accordingly)

.PHONY: sign
sign:
	@echo ">> Building the kernel"
	make
	@echo ">> Signing the binary"
	${PYTHON_EXECUTABLE} $(POST_BUILD_SCRIPT) -i $(TARGET_PATH)/release/$(PLATFORM).bin -o $(TARGET_PATH)/release/$(PLATFORM)_sign.bin -v
