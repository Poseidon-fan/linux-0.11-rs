#!/usr/bin/env python3
"""Convert an ELF32 executable to ZMAGIC a.out format.

Usage: elf2aout.py <input.elf> <output.aout>

Parses the ELF32 program headers to locate the text (R+X) and data (R+W)
LOAD segments, then emits an a.out file with:
  - 32-byte AoutHeader (padded to 1024 bytes)
  - Text segment content
  - Data segment content
"""

import struct
import sys

ZMAGIC = 0o413
BLOCK_SIZE = 1024
AOUT_HEADER_SIZE = 32

# ELF constants
PT_LOAD = 1
PF_X = 0x1
PF_W = 0x2


def die(msg):
    print(f"elf2aout: error: {msg}", file=sys.stderr)
    sys.exit(1)


def parse_elf32(data):
    # ELF32 header: 52 bytes
    if len(data) < 52:
        die("file too small for ELF header")
    if data[:4] != b"\x7fELF":
        die("not an ELF file")
    if data[4] != 1:
        die("not a 32-bit ELF (EI_CLASS != ELFCLASS32)")
    if data[5] != 1:
        die("not little-endian (EI_DATA != ELFDATA2LSB)")

    (e_type, e_machine, e_version, e_entry,
     e_phoff, e_shoff, e_flags, e_ehsize,
     e_phentsize, e_phnum, e_shentsize, e_shnum,
     e_shstrndx) = struct.unpack_from("<HHIIIIIHHHHHH", data, 16)

    if e_type != 2:
        die(f"not an executable (e_type={e_type}, expected ET_EXEC=2)")

    # Parse program headers
    segments = []
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        (p_type, p_offset, p_vaddr, p_paddr,
         p_filesz, p_memsz, p_flags, p_align) = struct.unpack_from(
            "<IIIIIIII", data, off)
        segments.append({
            "type": p_type,
            "offset": p_offset,
            "vaddr": p_vaddr,
            "filesz": p_filesz,
            "memsz": p_memsz,
            "flags": p_flags,
        })

    return e_entry, segments


def main():
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <input.elf> <output.aout>", file=sys.stderr)
        sys.exit(1)

    elf_path, aout_path = sys.argv[1], sys.argv[2]

    with open(elf_path, "rb") as f:
        elf_data = f.read()

    entry, segments = parse_elf32(elf_data)

    # Find LOAD segments
    loads = [s for s in segments if s["type"] == PT_LOAD]
    if not loads:
        die("no PT_LOAD segments found")

    # Sort by virtual address
    loads.sort(key=lambda s: s["vaddr"])

    # Identify text and data segments by flags
    text_seg = None
    data_seg = None

    for seg in loads:
        if seg["flags"] & PF_X:
            text_seg = seg
        elif seg["flags"] & PF_W:
            data_seg = seg

    if text_seg is None:
        die("no executable (PF_X) LOAD segment found")

    # Extract sizes.
    #
    # For ZMAGIC a.out, a_text covers the virtual address range from 0 up to
    # the data segment start.  When the linker page-aligns the text/data
    # boundary, a_text includes the alignment padding so that the on-disk
    # layout mirrors the in-memory layout (virtual address V maps to file
    # offset BLOCK_SIZE + V).
    text_raw = elf_data[text_seg["offset"]:text_seg["offset"] + text_seg["filesz"]]

    if data_seg is not None:
        a_text = data_seg["vaddr"]
        text_content = text_raw.ljust(a_text, b"\x00")

        a_data = data_seg["filesz"]
        a_bss = data_seg["memsz"] - data_seg["filesz"]
        data_content = elf_data[data_seg["offset"]:data_seg["offset"] + a_data]
    else:
        a_text = text_seg["filesz"]
        text_content = text_raw

        a_data = 0
        a_bss = 0
        data_content = b""

    a_entry = entry

    # Build a.out header
    header = struct.pack("<IIIIIIII",
                         ZMAGIC,     # a_magic
                         a_text,     # a_text
                         a_data,     # a_data
                         a_bss,      # a_bss
                         0,          # a_syms
                         a_entry,    # a_entry
                         0,          # a_trsize
                         0)          # a_drsize

    # Pad header to BLOCK_SIZE
    header_block = header + b"\x00" * (BLOCK_SIZE - AOUT_HEADER_SIZE)

    with open(aout_path, "wb") as f:
        f.write(header_block)
        f.write(text_content)
        f.write(data_content)

    total = BLOCK_SIZE + len(text_content) + len(data_content)
    print(f"  {aout_path}: text={a_text} data={a_data} bss={a_bss} "
          f"entry=0x{a_entry:x} total={total}")


if __name__ == "__main__":
    main()
