TARGET    := i386-unknown-none
AOUT_DIR  := aout
MODE      ?= release

ifeq ($(MODE),release)
  CARGO_FLAGS := --release
  BUILD_DIR   := target/$(TARGET)/release
else
  CARGO_FLAGS :=
  BUILD_DIR   := target/$(TARGET)/debug
endif

# Discover all binaries from src/bin/*.rs
ALL_BINS := $(patsubst src/bin/%.rs,%,$(wildcard src/bin/*.rs))

# When specific targets are given on the command line (e.g. `make test1 test2`),
# build only those; otherwise build everything.
REQUESTED := $(filter $(ALL_BINS),$(MAKECMDGOALS))
BINS      := $(if $(REQUESTED),$(REQUESTED),$(ALL_BINS))

.PHONY: all clean $(ALL_BINS)

all: $(BINS)

$(ALL_BINS):
	@cargo build $(CARGO_FLAGS) --bin $@
	@mkdir -p $(AOUT_DIR)
	@python3 scripts/elf2aout.py $(BUILD_DIR)/$@ $(AOUT_DIR)/$@

clean:
	@cargo clean
	@rm -rf $(AOUT_DIR)
