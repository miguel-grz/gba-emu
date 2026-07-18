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
//! * `IntrWait`/`VBlankIntrWait` currently just halt the CPU until an
//!   interrupt is pending. The BIOS's per-source flag bookkeeping (the
//!   accumulator at `0x0300_7FF8`) is completed once interrupt *sources*
//!   exist (Phase 3 PPU / Phase 6 timers+DMA); with none present today there
//!   is nothing yet to wait on.
//! * Decompression (LZ77/Huffman/RLE/diff), `BitUnpack`, and the affine-set
//!   helpers are not implemented yet and return without touching state; a
//!   game that calls them under HLE will misbehave and should use LLE until
//!   these land.

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
        // Unimplemented services (affine-set, decompression, BitUnpack, …).
        // Returning leaves state untouched rather than corrupting it.
        _ => {}
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
