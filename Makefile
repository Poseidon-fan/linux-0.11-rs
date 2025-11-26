TOOLCHAIN_PREFIX := x86_64-linux-gnu-
AS := $(TOOLCHAIN_PREFIX)as
LD := $(TOOLCHAIN_PREFIX)ld
OBJCOPY := $(TOOLCHAIN_PREFIX)objcopy
STRIP := $(TOOLCHAIN_PREFIX)strip

TARGET_TRIPLE := i386-unknown-none
TARGET_DIR := target/$(TARGET_TRIPLE)
SRC_DIR := src
BOOT_DIR := boot

KERNEL_BINARY_debug   = $(TARGET_DIR)/debug/linux_rs
KERNEL_BINARY_release = $(TARGET_DIR)/release/linux_rs
SYSTEM_BINARY := $(TARGET_DIR)/system
KERNEL_IMAGE := $(TARGET_DIR)/kernel
BOOTSECT_BINARY := $(TARGET_DIR)/bootsect
SETUP_BINARY := $(TARGET_DIR)/setup
BOOTSECT_SRC := $(SRC_DIR)/boot/bootsect.s
SETUP_SRC := $(SRC_DIR)/boot/setup.s
BUILD_SCRIPT := scripts/build.sh

ASFLAGS := --32
LDFLAGS := -m elf_i386 -Ttext 0
OBJCOPY_BOOT_FLAGS := -R .pdr -R .comment -R.note -S -O binary
OBJCOPY_KERNEL_FLAGS := -O binary -R .note -R .comment

all: Image-release

Image-%: $(BOOT_DIR)/bootsect $(BOOT_DIR)/setup
	@echo "Building kernel ($*)..."
	@cargo build $(if $(filter $*,release),--release,)
	@cp $(KERNEL_BINARY_$*) $(SYSTEM_BINARY)
	@$(STRIP) $(SYSTEM_BINARY)
	@$(OBJCOPY) $(OBJCOPY_KERNEL_FLAGS) $(SYSTEM_BINARY) $(KERNEL_IMAGE)
	@bash $(BUILD_SCRIPT) $(BOOTSECT_BINARY) $(SETUP_BINARY) $(KERNEL_IMAGE) Image-$*

$(BOOT_DIR)/%: $(SRC_DIR)/boot/%.s
	@echo "Building $@..."
	@mkdir -p $(TARGET_DIR)
	@$(AS) $(ASFLAGS) -o $(TARGET_DIR)/$*.o $<
	@$(LD) $(LDFLAGS) -o $(TARGET_DIR)/$* $(TARGET_DIR)/$*.o
	@$(OBJCOPY) $(OBJCOPY_BOOT_FLAGS) $(TARGET_DIR)/$* $(TARGET_DIR)/$*

run: Image-release
	@qemu-system-i386 -m 16M -boot a -fda Image-release -display curses

dbg: Image-debug
	@qemu-system-i386 -m 16M -boot a -fda Image-debug -display curses -s -S

clean:
	@echo "Cleaning..."
	@cargo clean
	@rm -f $(BOOTSECT_BINARY) $(SETUP_BINARY) $(KERNEL_IMAGE) $(SYSTEM_BINARY) Image

.PHONY: all clean Image
