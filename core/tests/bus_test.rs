//! Phase 2 tests: memory map, I/O registers, waitstate timing, BIOS HLE,
//! open bus, the 8-bit-write quirks, and interrupt/halt behavior.

use gba_core::bios;
use gba_core::io::irq;
use gba_core::memory::Bus;
use gba_core::timing::{access_cycles, Region, SeqTracker, Width};
use gba_core::{Cpu, Memory, Mode};

fn mem(rom: &[u32]) -> Memory {
    let mut bytes = Vec::new();
    for op in rom {
        bytes.extend_from_slice(&op.to_le_bytes());
    }
    Memory::new(bytes).expect("rom fits")
}

const IE: u32 = 0x0400_0200;
const IF: u32 = 0x0400_0202;
const IME: u32 = 0x0400_0208;
const WAITCNT: u32 = 0x0400_0204;
const HALTCNT: u32 = 0x0400_0301;

// ----------------------------------------------------------- I/O registers

#[test]
fn io_registers_read_back() {
    let mut m = mem(&[]);
    m.write16(IE, 0x1234);
    m.write16(IME, 0x0001);
    m.write16(WAITCNT, 0x4317);
    assert_eq!(m.read16(IE), 0x1234);
    assert_eq!(m.read16(IME), 0x0001);
    assert_eq!(m.read16(WAITCNT), 0x4317);
}

#[test]
fn if_is_write_one_to_clear() {
    let mut m = mem(&[]);
    m.raise_irq(irq::VBLANK | irq::TIMER0 | irq::DMA1);
    assert_eq!(m.read16(IF), irq::VBLANK | irq::TIMER0 | irq::DMA1);
    // Writing a 1 acknowledges (clears) only those bits.
    m.write16(IF, irq::TIMER0);
    assert_eq!(m.read16(IF), irq::VBLANK | irq::DMA1);
}

#[test]
fn keyinput_reads_as_no_keys_pressed() {
    let mut m = mem(&[]);
    assert_eq!(m.read16(0x0400_0130), 0x03FF);
}

#[test]
fn irq_pending_requires_enable_and_master() {
    let mut m = mem(&[]);
    m.raise_irq(irq::VBLANK);
    assert!(!m.irq_pending(), "no IE, no IME");
    m.write16(IE, irq::VBLANK);
    assert!(!m.irq_pending(), "IE set but IME clear");
    m.write16(IME, 1);
    assert!(m.irq_pending(), "IE + IME + IF");
    // A different enabled source must not spuriously fire.
    m.write16(IE, irq::HBLANK);
    assert!(!m.irq_pending());
}

// --------------------------------------------------------- 8-bit-write quirks

#[test]
fn palette_byte_write_duplicates_across_halfword() {
    let mut m = mem(&[]);
    m.write8(0x0500_0000, 0xAB);
    // The byte is mirrored into both halves of the 16-bit cell.
    assert_eq!(m.read16(0x0500_0000), 0xABAB);
}

#[test]
fn vram_byte_write_duplicates_in_bg_area() {
    let mut m = mem(&[]);
    m.write8(0x0600_0000, 0xCD);
    assert_eq!(m.read16(0x0600_0000), 0xCDCD);
}

#[test]
fn oam_ignores_byte_writes() {
    let mut m = mem(&[]);
    m.write16(0x0700_0000, 0x1111);
    m.write8(0x0700_0000, 0xFF); // ignored
    assert_eq!(m.read16(0x0700_0000), 0x1111);
}

// ------------------------------------------------------------------ open bus

#[test]
fn out_of_bounds_rom_reads_open_bus_pattern() {
    // A 4 KiB ROM; reads past the end float with the addr/2 prefetch pattern.
    let mut m = Memory::new(vec![0; 0x1000]).unwrap();
    let addr = 0x0800_2468;
    assert_eq!(m.read16(addr), (addr >> 1) as u16);
}

// -------------------------------------------------------- waitstate timing

#[test]
fn timing_fast_regions_are_single_cycle() {
    assert_eq!(access_cycles(Region::Fast, Width::Word, false, 0), 1);
    assert_eq!(access_cycles(Region::Fast, Width::Byte, true, 0), 1);
}

#[test]
fn timing_ewram_has_two_waitstates() {
    assert_eq!(access_cycles(Region::Ewram, Width::Byte, false, 0), 3);
    assert_eq!(access_cycles(Region::Ewram, Width::Word, false, 0), 6);
}

#[test]
fn timing_video_is_16bit_bus() {
    assert_eq!(access_cycles(Region::Video, Width::Half, false, 0), 1);
    assert_eq!(access_cycles(Region::Video, Width::Word, false, 0), 2);
}

#[test]
fn timing_rom_uses_waitcnt_n_and_s() {
    // WAITCNT = 0: WS0 N = 4 waits (5 cycles), S = 2 waits (3 cycles).
    assert_eq!(access_cycles(Region::Rom0, Width::Half, false, 0), 5); // N
    assert_eq!(access_cycles(Region::Rom0, Width::Half, true, 0), 3); // S
    assert_eq!(access_cycles(Region::Rom0, Width::Word, false, 0), 8); // N + S
                                                                       // Set WS0 S=1 (bit 4) and WS0 N=2 (bits 3:2 = 0b10): N=3 cycles, S=2.
    let waitcnt = (0b10 << 2) | (1 << 4);
    assert_eq!(access_cycles(Region::Rom0, Width::Half, false, waitcnt), 3);
    assert_eq!(access_cycles(Region::Rom0, Width::Half, true, waitcnt), 2);
}

#[test]
fn seq_tracker_detects_sequential_runs() {
    let mut s = SeqTracker::default();
    assert!(!s.classify(0x0800_0000, Width::Word), "first access is N");
    assert!(s.classify(0x0800_0004, Width::Word), "contiguous is S");
    assert!(s.classify(0x0800_0008, Width::Word), "still S");
    assert!(!s.classify(0x0800_0100, Width::Word), "jump breaks the run");
    assert!(!s.classify(0x0300_0000, Width::Word), "region change is N");
}

#[test]
fn cpu_cycle_counter_reflects_bus_timing() {
    // Run a few IWRAM instructions and confirm the CPU accrued real cycles
    // from the bus (fast region → at least one fetch cycle per instruction).
    let mut m = mem(&[
        0xE3A00001, // mov r0, #1
        0xE3A01002, // mov r1, #2
        0xEAFFFFFE, // b .
    ]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    let before = cpu.cycles();
    cpu.step(&mut m);
    cpu.step(&mut m);
    assert!(
        cpu.cycles() > before,
        "cycle counter advanced from bus timing"
    );
}

// ------------------------------------------------------------- BIOS HLE

/// Execute a single ARM `SWI number` from ROM (HLE mode) after setting up
/// registers, and return the resulting CPU.
fn run_swi(number: u8, setup: impl FnOnce(&mut Cpu)) -> (Cpu, Memory) {
    let swi = 0xEF00_0000 | (u32::from(number) << 16);
    let mut m = mem(&[swi, 0xEAFF_FFFE]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    setup(&mut cpu);
    cpu.step(&mut m);
    (cpu, m)
}

#[test]
fn hle_div_computes_quotient_remainder_and_abs() {
    let (cpu, _) = run_swi(0x06, |cpu| {
        cpu.set_reg(0, 100u32);
        cpu.set_reg(1, 7u32);
    });
    assert_eq!(cpu.reg(0), 14); // quotient
    assert_eq!(cpu.reg(1), 2); // remainder
    assert_eq!(cpu.reg(3), 14); // |quotient|
}

#[test]
fn hle_div_signed_negative() {
    let (cpu, _) = run_swi(0x06, |cpu| {
        cpu.set_reg(0, (-100i32) as u32);
        cpu.set_reg(1, 7u32);
    });
    assert_eq!(cpu.reg(0) as i32, -14);
    assert_eq!(cpu.reg(1) as i32, -2);
    assert_eq!(cpu.reg(3), 14);
}

#[test]
fn hle_div_by_zero_is_defined() {
    let (cpu, _) = run_swi(0x06, |cpu| {
        cpu.set_reg(0, 5u32);
        cpu.set_reg(1, 0u32);
    });
    assert_eq!(cpu.reg(0) as i32, 1); // sign of numerator
    assert_eq!(cpu.reg(1), 5); // numerator
    assert_eq!(cpu.reg(3), 1);
}

#[test]
fn hle_divarm_swaps_operands() {
    // DivArm (0x07): r0 = denominator, r1 = numerator.
    let (cpu, _) = run_swi(0x07, |cpu| {
        cpu.set_reg(0, 7u32);
        cpu.set_reg(1, 100u32);
    });
    assert_eq!(cpu.reg(0), 14);
    assert_eq!(cpu.reg(1), 2);
}

#[test]
fn hle_sqrt() {
    let (cpu, _) = run_swi(0x08, |cpu| cpu.set_reg(0, 1_000_000));
    assert_eq!(cpu.reg(0), 1000);
    let (cpu, _) = run_swi(0x08, |cpu| cpu.set_reg(0, 2));
    assert_eq!(cpu.reg(0), 1);
}

#[test]
fn hle_get_bios_checksum() {
    let (cpu, _) = run_swi(0x0D, |_| {});
    assert_eq!(cpu.reg(0), 0xBAAE_187F);
}

#[test]
fn hle_arctan2_quadrants() {
    // atan2(+1, +1) ≈ 45° = 0x2000 in the 0x10000 = 360° scale.
    let (cpu, _) = run_swi(0x0A, |cpu| {
        cpu.set_reg(0, (1 << 14) as u32); // x = 1.0
        cpu.set_reg(1, (1 << 14) as u32); // y = 1.0
    });
    let angle = cpu.reg(0);
    assert!((0x1F00..=0x2100).contains(&angle), "got {angle:#06X}");
}

#[test]
fn hle_cpu_set_word_copy() {
    let mut m = mem(&[]);
    // Seed 4 words in EWRAM, copy them 0x100 bytes further on.
    for i in 0..4u32 {
        m.write32(0x0200_0000 + i * 4, 0x1000 + i);
    }
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    cpu.set_reg(0, 0x0200_0000); // src
    cpu.set_reg(1, 0x0200_0100); // dst
    cpu.set_reg(2, 4 | 1 << 26); // count 4, 32-bit
    bios::hle_swi(&mut cpu, &mut m, 0x0B);
    for i in 0..4u32 {
        assert_eq!(m.read32(0x0200_0100 + i * 4), 0x1000 + i);
    }
}

#[test]
fn hle_cpu_set_fill_mode() {
    let mut m = mem(&[]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    m.write16(0x0200_0000, 0xBEEF);
    cpu.set_reg(0, 0x0200_0000); // src (fixed)
    cpu.set_reg(1, 0x0200_0010); // dst
    cpu.set_reg(2, 4 | 1 << 24); // count 4 halfwords, fixed source
    bios::hle_swi(&mut cpu, &mut m, 0x0B);
    for i in 0..4u32 {
        assert_eq!(m.read16(0x0200_0010 + i * 2), 0xBEEF);
    }
}

#[test]
fn hle_register_ram_reset_clears_ewram() {
    let mut m = mem(&[]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    m.write32(0x0200_0000, 0xDEAD_BEEF);
    cpu.set_reg(0, 1); // bit 0 = clear EWRAM
    bios::hle_swi(&mut cpu, &mut m, 0x01);
    assert_eq!(m.read32(0x0200_0000), 0);
}

// ------------------------------------------------------- interrupts & halt

#[test]
fn irq_dispatched_through_bus() {
    let mut m = mem(&[0xEAFF_FFFE]); // b . at ROM start
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m); // System mode, IRQs enabled
    m.write16(IE, irq::VBLANK);
    m.write16(IME, 1);
    m.raise_irq(irq::VBLANK);
    cpu.step(&mut m);
    assert_eq!(cpu.mode(), Mode::Irq);
    assert_eq!(cpu.pc(), 0x18); // IRQ vector
}

#[test]
fn halt_idles_until_interrupt() {
    // SWI 0x02 (Halt) under HLE puts the CPU to sleep.
    let mut m = mem(&[0xEF02_0000, 0xEAFF_FFFE]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    cpu.step(&mut m); // executes Halt
    assert!(cpu.is_halted());
    let pc = cpu.pc();
    cpu.step(&mut m); // idles, no progress
    assert!(cpu.is_halted());
    assert_eq!(cpu.pc(), pc);
    // An enabled interrupt wakes it and is serviced.
    m.write16(IE, irq::VBLANK);
    m.write16(IME, 1);
    m.raise_irq(irq::VBLANK);
    cpu.step(&mut m);
    assert!(!cpu.is_halted());
    assert_eq!(cpu.mode(), Mode::Irq);
}

#[test]
fn haltcnt_write_halts_in_lle_path() {
    // Writing HALTCNT via the I/O register also halts the CPU (LLE path).
    let mut m = mem(&[0xE3A0_0000, 0xEAFF_FFFE]); // mov r0,#0 ; b .
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    m.write8(HALTCNT, 0x80);
    cpu.step(&mut m);
    assert!(cpu.is_halted());
}

// -------------------------------------------------- BIOS decompression SWIs

const EWRAM_SRC: u32 = 0x0200_0000;
const EWRAM_DST: u32 = 0x0200_0100;

fn decompress(number: u8, header: u32, payload: &[u8]) -> Vec<u8> {
    let mut m = mem(&[]);
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m);
    m.write32(EWRAM_SRC, header);
    for (i, &b) in payload.iter().enumerate() {
        m.write8(EWRAM_SRC + 4 + i as u32, b);
    }
    cpu.set_reg(0, EWRAM_SRC);
    cpu.set_reg(1, EWRAM_DST);
    bios::hle_swi(&mut cpu, &mut m, number);
    let size = (header >> 8) as usize;
    (0..size as u32).map(|i| m.read8(EWRAM_DST + i)).collect()
}

#[test]
fn lz77_decompresses_literals_and_backrefs() {
    // 8 bytes: literal 'A', then copy 7 more from displacement 1.
    let out = decompress(0x11, (8 << 8) | 0x10, &[0x40, 0x41, 0x40, 0x00]);
    assert_eq!(out, vec![0x41; 8]);
}

#[test]
fn rle_decompresses_runs() {
    // A compressed run of 5 'B' bytes (flag 0x82 = run, len (2)+3).
    let out = decompress(0x14, (5 << 8) | 0x30, &[0x82, 0x42]);
    assert_eq!(out, vec![0x42; 5]);
}

#[test]
fn diff8_unfilter_accumulates() {
    // Differences 10, +1, +1, +1 -> 10, 11, 12, 13.
    let out = decompress(0x16, (4 << 8) | 0x81, &[10, 1, 1, 1]);
    assert_eq!(out, vec![10, 11, 12, 13]);
}

// ---------------------------------------------- HLE BIOS interrupt dispatch

#[test]
fn hle_irq_handler_calls_game_handler() {
    // A game-style setup: an IRQ handler that acknowledges IF and bumps a
    // counter, reached through the injected BIOS dispatcher.
    let mut m = mem(&[0xEAFF_FFFE]); // main code: b .
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut m); // System mode, IRQs enabled

    // Game IRQ handler in EWRAM at 0x0200_0200.
    let handler: [u32; 9] = [
        0xE3A0_0301, // mov  r0, #0x0400_0000
        0xE280_0C02, // add  r0, r0, #0x200
        0xE3A0_1001, // mov  r1, #1
        0xE1C0_10B2, // strh r1, [r0, #2]   ; ack IF (vblank)
        0xE3A0_2402, // mov  r2, #0x0200_0000
        0xE592_3000, // ldr  r3, [r2]
        0xE283_3001, // add  r3, r3, #1
        0xE582_3000, // str  r3, [r2]       ; counter++
        0xE12F_FF1E, // bx   lr
    ];
    for (i, &w) in handler.iter().enumerate() {
        m.write32(0x0200_0200 + i as u32 * 4, w);
    }
    m.write32(0x0300_7FFC, 0x0200_0200); // BIOS user IRQ handler pointer
    m.write16(IE, irq::VBLANK);
    m.write16(IME, 1);

    m.raise_irq(irq::VBLANK);
    for _ in 0..40 {
        cpu.step(&mut m);
    }
    assert_eq!(m.read32(0x0200_0000), 1, "the game's IRQ handler ran once");
}
