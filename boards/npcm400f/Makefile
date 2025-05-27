# Licensed under the Apache License, Version 2.0 or the MIT License.
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Copyright Tock Contributors 2022.

# Makefile for building the tock kernel for the NPCM400 platform
#

include ../Makefile.common

OPENOCD=openocd
POST_BUILD_SCRIPT=$(TOCK_ROOT_DIRECTORY)boards/npcm400f/esiost/esiost.py
SIGNED_IMG_PATH=$(TARGET_PATH)/release/$(PLATFORM)_sign
KERNEL_ADDRESS=0x00000600
TBF_BIN=/home/alan/workspace/libtock-rs/target/tbf/npcm400/console.tbf

OBJCOPY_FLAGS_NPCM400 ?= --strip-sections --strip-all

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

	@echo ">> Set section .apps to LOAD,ALLOC"
	# $(OBJCOPY) --set-section-flags .apps=LOAD,ALLOC $(TARGET_PATH)/release/$(PLATFORM).elf

	@echo ">> Copy app .tbf to section .apps"
	# $(OBJCOPY) --update-section .apps=$(TBF_BIN) $(TARGET_PATH)/release/$(PLATFORM).elf $(TARGET_PATH)/release/$(PLATFORM).elf

	@echo ">> Make binary"
	# $(OBJCOPY) --output-target=binary $(OBJCOPY_FLAGS) $(TARGET_PATH)/release/$(PLATFORM).elf $(TARGET_PATH)/release/$(PLATFORM).bin

	@echo ">> dump elf"
	# $(OBJDUMP) -r -x $(TARGET_PATH)/release/$(PLATFORM).elf > /home/alan/workspace/tock/objdump.txt
	# $(OBJDUMP) --disassemble-all --source --section-headers --demangle $(TARGET_PATH)/release/$(PLATFORM).elf > /home/alan/workspace/tock/objdump.txt
	$(OBJDUMP) -r -x --source --section-headers --demangle $(TARGET_PATH)/release/$(PLATFORM).elf > /home/alan/workspace/tock/objdump.txt

	@echo ">> Signing the binary"
	${PYTHON_EXECUTABLE} $(POST_BUILD_SCRIPT) -i $(TARGET_PATH)/release/$(PLATFORM).bin -o $(SIGNED_IMG_PATH).bin -v
