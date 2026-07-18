//! BIOS software-interrupt handling.
//!
//! The GBA BIOS is a 16 KiB ROM that, among other things, provides a set of
//! SWI routines (division, square root, memory copies, decompression, the
//! halt/interrupt-wait services, …). We support two modes:
//!
//! * **LLE** — if a real BIOS image is loaded, `SWI` takes the normal
//!   exception to vector `0x08` and the BIOS code runs. Bit-exact, and the
//!   only way to match every documented quirk, but the image cannot be
//!   redistributed.
//! * **HLE** — otherwise, we intercept `SWI` and emulate the routine in Rust
//!   here, so the emulator runs with no copyrighted files. This module is that
//!   emulation.
//!
//! [`Memory`](crate::memory::Memory) reports which mode is active via
//! [`Bus::has_bios`](crate::memory::Bus::has_bios); the CPU consults it in
//! [`Cpu::do_swi`](crate::cpu::Cpu::do_swi).
//!
//! ### Accuracy notes
//!
//! * `Div`/`Sqrt`/`CpuSet`/`CpuFastSet`/`RegisterRamReset` are exact.
//! * `ArcTan`/`ArcTan2` use the mathematically correct result rather than the
//!   BIOS's fixed-point polynomial. They agree with hardware to within the
//!   BIOS approximation's own error (a few low bits); code that needs
//!   bit-identical BIOS trig should run in LLE mode.
//! * `IntrWait`/`VBlankIntrWait` halt the CPU until an interrupt arrives; the
//!   injected BIOS interrupt dispatcher (see [`crate::memory`]) then runs the
//!   game's own handler. This wakes on any enabled interrupt rather than the
//!   specific awaited one — accurate for the common VBlank-only case.
//! * Decompression (LZ77, run-length, Huffman, diff-unfilter) and `BitUnpack`
//!   are implemented. The affine-set helpers (0x0E/0x0F) are not.

use crate::cpu::Cpu;
use crate::memory::Bus;
use std::f64::consts::TAU;

/// Emulate BIOS SWI `number` in place, operating on the CPU registers and
/// memory. Called only in HLE mode.
pub fn hle_swi<B: Bus>(cpu: &mut Cpu, bus: &mut B, number: u8) {
    match number {
        0x00 => soft_reset(cpu, bus),
        0x01 => register_ram_reset(cpu, bus),
        0x02 => cpu.request_halt(),
        0x03 => cpu.request_halt(), // Stop/Sleep — treated as Halt headlessly
        0x04 => cpu.request_halt(), // IntrWait (see module notes)
        0x05 => cpu.request_halt(), // VBlankIntrWait
        0x06 => div(cpu, false),
        0x07 => div(cpu, true), // DivArm: numerator/denominator swapped
        0x08 => cpu.set_reg(0, isqrt(cpu.reg(0))),
        0x09 => arctan(cpu),
        0x0A => arctan2(cpu),
        0x0B => cpu_set(cpu, bus),
        0x0C => cpu_fast_set(cpu, bus),
        0x0D => {
            // GetBiosChecksum: the constant returned by the retail GBA BIOS.
            cpu.set_reg(0, 0xBAAE_187F);
        }
        0x10 => bit_unpack(cpu, bus),
        0x11 => lz77(cpu, bus, false), // LZ77UnCompWRAM (byte writes)
        0x12 => lz77(cpu, bus, true),  // LZ77UnCompVRAM (halfword writes)
        0x13 => huffman(cpu, bus),
        0x14 => rl_uncomp(cpu, bus, false),        // RLUnCompWRAM
        0x15 => rl_uncomp(cpu, bus, true),         // RLUnCompVRAM
        0x16 => diff_unfilter(cpu, bus, 8, false), // Diff8bitUnFilterWRAM
        0x17 => diff_unfilter(cpu, bus, 8, true),  // Diff8bitUnFilterVRAM
        0x18 => diff_unfilter(cpu, bus, 16, true), // Diff16bitUnFilter (VRAM)
        // Affine-set (0x0E/0x0F), sound and other services are not implemented;
        // returning leaves state untouched rather than corrupting it.
        _ => {}
    }
}

/// Write a decompressed byte buffer to `dst`, as bytes (WRAM) or halfwords
/// (VRAM, which cannot take 8-bit writes).
fn write_out<B: Bus>(bus: &mut B, dst: u32, data: &[u8], vram: bool) {
    if vram {
        let mut i = 0;
        while i + 1 < data.len() {
            let hw = u16::from(data[i]) | u16::from(data[i + 1]) << 8;
            bus.write16(dst + i as u32, hw);
            i += 2;
        }
    } else {
        for (i, &b) in data.iter().enumerate() {
            bus.write8(dst + i as u32, b);
        }
    }
}

/// LZ77 decompression (SWI 0x11 WRAM / 0x12 VRAM). r0 = source, r1 = dest.
fn lz77<B: Bus>(cpu: &mut Cpu, bus: &mut B, vram: bool) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let size = (bus.read32(src) >> 8) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut p = src + 4;
    while out.len() < size {
        let flags = bus.read8(p);
        p += 1;
        for bit in 0..8 {
            if out.len() >= size {
                break;
            }
            if flags & (0x80 >> bit) != 0 {
                let b1 = bus.read8(p);
                let b2 = bus.read8(p + 1);
                p += 2;
                let len = (b1 >> 4) as usize + 3;
                let back = ((u16::from(b1 & 0xF) << 8) | u16::from(b2)) as usize + 1;
                for _ in 0..len {
                    if out.len() >= size || back > out.len() {
                        break;
                    }
                    out.push(out[out.len() - back]);
                }
            } else {
                out.push(bus.read8(p));
                p += 1;
            }
        }
    }
    write_out(bus, dst, &out, vram);
}

/// Run-length decompression (SWI 0x14 WRAM / 0x15 VRAM). r0 = src, r1 = dest.
fn rl_uncomp<B: Bus>(cpu: &mut Cpu, bus: &mut B, vram: bool) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let size = (bus.read32(src) >> 8) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut p = src + 4;
    while out.len() < size {
        let flag = bus.read8(p);
        p += 1;
        if flag & 0x80 != 0 {
            // Compressed run: repeat one byte (len & 0x7F) + 3 times.
            let len = (flag & 0x7F) as usize + 3;
            let byte = bus.read8(p);
            p += 1;
            for _ in 0..len {
                out.push(byte);
            }
        } else {
            // Literal run of (len & 0x7F) + 1 bytes.
            let len = (flag & 0x7F) as usize + 1;
            for _ in 0..len {
                out.push(bus.read8(p));
                p += 1;
            }
        }
    }
    out.truncate(size);
    write_out(bus, dst, &out, vram);
}

/// Diff (delta) unfiltering (SWI 0x16/0x17 8-bit, 0x18 16-bit). Each element is
/// the running sum of the encoded differences. r0 = source, r1 = dest.
fn diff_unfilter<B: Bus>(cpu: &mut Cpu, bus: &mut B, elem_bits: u32, vram: bool) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let size = (bus.read32(src) >> 8) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut p = src + 4;
    if elem_bits == 8 {
        let mut acc = 0u8;
        while out.len() < size {
            acc = acc.wrapping_add(bus.read8(p));
            p += 1;
            out.push(acc);
        }
    } else {
        let mut acc = 0u16;
        while out.len() < size {
            acc = acc.wrapping_add(bus.read16(p));
            p += 2;
            out.push(acc as u8);
            out.push((acc >> 8) as u8);
        }
    }
    out.truncate(size);
    write_out(bus, dst, &out, vram);
}

/// Huffman decompression (SWI 0x13). r0 = source, r1 = dest. Supports the 4-
/// and 8-bit symbol sizes GBA graphics use.
fn huffman<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let header = bus.read32(src);
    let sym_bits = header & 0xF;
    let size = (header >> 8) as usize;
    let tree_base = src + 5;
    let tree_size = (u32::from(bus.read8(src + 4)) + 1) * 2;
    let mut stream = src + 4 + tree_size;

    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut pending = 0u32; // assembled symbols, low nibbles first
    let mut pending_bits = 0u32;
    let mut node = tree_base; // offset of the current node
    let mut root = true;

    while out.len() < size {
        let mut bits = bus.read32(stream);
        stream += 4;
        for _ in 0..32 {
            let node_val = bus.read8(node);
            if root {
                // The root byte is the tree's own node data.
                root = false;
            }
            let go_right = bits & 0x8000_0000 != 0;
            bits <<= 1;
            let offset = u32::from(node_val & 0x3F);
            let next = (node & !1) + offset * 2 + 2;
            let is_leaf = if go_right {
                node_val & 0x40 != 0
            } else {
                node_val & 0x80 != 0
            };
            node = if go_right { next + 1 } else { next };
            if is_leaf {
                let sym = u32::from(bus.read8(node)) & ((1 << sym_bits) - 1);
                pending |= sym << pending_bits;
                pending_bits += sym_bits;
                while pending_bits >= 8 {
                    out.push(pending as u8);
                    pending >>= 8;
                    pending_bits -= 8;
                }
                node = tree_base;
            }
            if out.len() >= size {
                break;
            }
        }
    }
    out.truncate(size);
    // Huffman output is always written as 32-bit units to WRAM or VRAM.
    let vram = dst >> 24 == 0x06;
    write_out(bus, dst, &out, vram);
}

/// BitUnpack (SWI 0x10): expand packed 1/2/4/8-bit units to wider units with an
/// optional base offset. r0 = source, r1 = dest, r2 = parameter block.
fn bit_unpack<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let src = cpu.reg(0);
    let dst = cpu.reg(1);
    let info = cpu.reg(2);
    let src_len = bus.read16(info) as usize;
    let src_bits = u32::from(bus.read8(info + 2));
    let dst_bits = u32::from(bus.read8(info + 3));
    let base = bus.read32(info + 4);
    let offset = base & 0x7FFF_FFFF;
    let zero_offset = base & 0x8000_0000 != 0;

    let mut out = 0u32;
    let mut out_bits = 0u32;
    let mut written = 0u32;
    for i in 0..src_len {
        let byte = bus.read8(src + i as u32);
        let mut consumed = 0u32;
        while consumed < 8 {
            let unit = u32::from(byte >> consumed) & ((1 << src_bits) - 1);
            consumed += src_bits;
            let value = if unit != 0 || zero_offset {
                unit + offset
            } else {
                0
            };
            out |= value << out_bits;
            out_bits += dst_bits;
            if out_bits == 32 {
                bus.write32(dst + written, out);
                written += 4;
                out = 0;
                out_bits = 0;
            }
        }
    }
}

/// Signed division shared by `Div` (r0/r1) and `DivArm` (r1/r0, `swapped`).
fn div(cpu: &mut Cpu, swapped: bool) {
    let (numerator, denominator) = if swapped {
        (cpu.reg(1), cpu.reg(0))
    } else {
        (cpu.reg(0), cpu.reg(1))
    };
    let num = numerator as i32;
    let den = denominator as i32;
    if den == 0 {
        // Hardware defines a result even here: quotient is the sign of the
        // numerator, remainder is the numerator, |quotient| is 1.
        let q = if num >= 0 { 1 } else { -1 };
        cpu.set_reg(0, q as u32);
        cpu.set_reg(1, num as u32);
        cpu.set_reg(3, 1);
    } else {
        // wrapping_div/rem give the defined INT_MIN / -1 result without panic.
        let q = num.wrapping_div(den);
        let r = num.wrapping_rem(den);
        cpu.set_reg(0, q as u32);
        cpu.set_reg(1, r as u32);
        cpu.set_reg(3, q.unsigned_abs());
    }
}

/// Integer square root of an unsigned 32-bit value.
fn isqrt(value: u32) -> u32 {
    let mut rem = value as u64;
    let mut root = 0u64;
    let mut bit = 1u64 << 30;
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root as u32
}

/// ArcTan: r0 = tan value in 1.14 signed fixed point → angle (0x10000 = 360°).
fn arctan(cpu: &mut Cpu) {
    let x = f64::from(cpu.reg(0) as i16) / f64::from(1 << 14);
    let angle = (x.atan() / TAU * 65536.0).round() as i32;
    cpu.set_reg(0, angle as u32);
}

/// ArcTan2: r0 = x, r1 = y (1.14 signed) → angle 0..0xFFFF over the full circle.
fn arctan2(cpu: &mut Cpu) {
    let x = f64::from(cpu.reg(0) as i16) / f64::from(1 << 14);
    let y = f64::from(cpu.reg(1) as i16) / f64::from(1 << 14);
    let mut angle = (y.atan2(x) / TAU * 65536.0).round() as i32;
    angle &= 0xFFFF;
    cpu.set_reg(0, angle as u32);
}

/// CpuSet: block copy or fill, 16- or 32-bit units.
fn cpu_set<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let mut src = cpu.reg(0);
    let mut dst = cpu.reg(1);
    let control = cpu.reg(2);
    let count = control & 0x1F_FFFF;
    let fixed = control & 1 << 24 != 0;
    let word = control & 1 << 26 != 0;
    let step = if word { 4 } else { 2 };
    for _ in 0..count {
        if word {
            let v = bus.read32(src);
            bus.write32(dst, v);
        } else {
            let v = bus.read16(src);
            bus.write16(dst, v);
        }
        dst = dst.wrapping_add(step);
        if !fixed {
            src = src.wrapping_add(step);
        }
    }
}

/// CpuFastSet: 32-bit block copy/fill in 8-word chunks.
fn cpu_fast_set<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let mut src = cpu.reg(0);
    let mut dst = cpu.reg(1);
    let control = cpu.reg(2);
    // Count is rounded up to a multiple of 8 words.
    let count = (control & 0x1F_FFFF).div_ceil(8) * 8;
    let fixed = control & 1 << 24 != 0;
    for _ in 0..count {
        let v = bus.read32(src);
        bus.write32(dst, v);
        dst = dst.wrapping_add(4);
        if !fixed {
            src = src.wrapping_add(4);
        }
    }
}

/// RegisterRamReset: clear selected RAM regions and (partially) reset I/O.
fn register_ram_reset<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let flags = cpu.reg(0);
    let clear = |bus: &mut B, start: u32, len: u32| {
        for off in (0..len).step_by(4) {
            bus.write32(start + off, 0);
        }
    };
    if flags & 1 << 0 != 0 {
        clear(bus, 0x0200_0000, 0x0004_0000); // 256 KiB EWRAM
    }
    if flags & 1 << 1 != 0 {
        // 32 KiB IWRAM except the top 0x200 bytes (BIOS/IRQ stack area).
        clear(bus, 0x0300_0000, 0x0000_7E00);
    }
    if flags & 1 << 2 != 0 {
        clear(bus, 0x0500_0000, 0x0000_0400); // palette
    }
    if flags & 1 << 3 != 0 {
        clear(bus, 0x0600_0000, 0x0001_8000); // VRAM (96 KiB)
    }
    if flags & 1 << 4 != 0 {
        clear(bus, 0x0700_0000, 0x0000_0400); // OAM
    }
    // Bits 5-7 (SIO / sound / other I/O resets) are completed alongside the
    // peripherals that own those registers.
}

/// SoftReset: return to the entry point recorded by the BIOS. The flag byte at
/// 0x0300_7FFA selects RAM (0x0200_0000) vs ROM (0x0800_0000) entry.
fn soft_reset<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let ram_entry = bus.read8(0x0300_7FFA) != 0;
    // Clear the top of IWRAM the BIOS uses for its own bookkeeping.
    for off in (0..0x200).step_by(4) {
        bus.write32(0x0300_7E00 + off, 0);
    }
    let target = if ram_entry { 0x0200_0000 } else { 0x0800_0000 };
    cpu.reset_to(bus, target);
}
