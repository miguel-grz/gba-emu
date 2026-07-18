//! Phase 6 tests: DMA channels, hardware timers, and their interrupts.

use gba_core::io::irq;
use gba_core::memory::Bus;
use gba_core::{Gba, Memory};

// DMA0 registers.
const DMA0_SAD: u32 = 0x0400_00B0;
const DMA0_DAD: u32 = 0x0400_00B4;
const DMA0_CNT: u32 = 0x0400_00B8; // CNT_L (low) + CNT_H (high)
                                   // Timer registers.
const TM0: u32 = 0x0400_0100;
const TM1: u32 = 0x0400_0104;
const IF: u32 = 0x0400_0202;

const EWRAM: u32 = 0x0200_0000;

// DMA control bits.
const DMA_ENABLE: u32 = 1 << 15;
const DMA_WORD: u32 = 1 << 10;
const DMA_IRQ: u32 = 1 << 14;
const DMA_REPEAT: u32 = 1 << 9;
const DMA_TIMING_VBLANK: u32 = 1 << 12;
const SRC_FIXED: u32 = 2 << 7;

fn blank() -> Memory {
    Memory::new(vec![0; 0x100]).unwrap()
}

// --------------------------------------------------------------------- DMA

#[test]
fn dma_immediate_word_copy() {
    let mut m = blank();
    for i in 0..4u32 {
        m.write32(EWRAM + i * 4, 0xA000 + i);
    }
    m.write32(DMA0_SAD, EWRAM);
    m.write32(DMA0_DAD, EWRAM + 0x100);
    // count = 4, control = enable | 32-bit | immediate.
    m.write32(DMA0_CNT, 4 | ((DMA_ENABLE | DMA_WORD) << 16));
    m.tick(0); // immediate DMA runs on the next tick

    for i in 0..4u32 {
        assert_eq!(m.read32(EWRAM + 0x100 + i * 4), 0xA000 + i);
    }
    // A non-repeating channel disables itself when done.
    assert_eq!(m.read16(0x0400_00BA) & 0x8000, 0);
}

#[test]
fn dma_immediate_halfword_copy() {
    let mut m = blank();
    for i in 0..8u32 {
        m.write16(EWRAM + i * 2, (0xB0 + i) as u16);
    }
    m.write32(DMA0_SAD, EWRAM);
    m.write32(DMA0_DAD, EWRAM + 0x100);
    m.write32(DMA0_CNT, 8 | (DMA_ENABLE << 16)); // 16-bit
    m.tick(0);

    for i in 0..8u32 {
        assert_eq!(m.read16(EWRAM + 0x100 + i * 2), (0xB0 + i) as u16);
    }
}

#[test]
fn dma_fixed_source_fills() {
    let mut m = blank();
    m.write32(EWRAM, 0xDEAD_BEEF);
    m.write32(DMA0_SAD, EWRAM);
    m.write32(DMA0_DAD, EWRAM + 0x100);
    m.write32(DMA0_CNT, 4 | ((DMA_ENABLE | DMA_WORD | SRC_FIXED) << 16));
    m.tick(0);

    for i in 0..4u32 {
        assert_eq!(m.read32(EWRAM + 0x100 + i * 4), 0xDEAD_BEEF);
    }
}

#[test]
fn dma_raises_completion_interrupt() {
    let mut m = blank();
    m.write32(DMA0_SAD, EWRAM);
    m.write32(DMA0_DAD, EWRAM + 0x100);
    m.write32(DMA0_CNT, 1 | ((DMA_ENABLE | DMA_WORD | DMA_IRQ) << 16));
    m.tick(0);
    assert_eq!(m.read16(IF) & irq::DMA0, irq::DMA0);
}

#[test]
fn dma_vblank_timing_and_repeat() {
    let mut m = blank();
    m.write32(EWRAM, 0x1234_5678);
    m.write32(DMA0_SAD, EWRAM);
    m.write32(DMA0_DAD, EWRAM + 0x100);
    // 1 word, enable | 32-bit | VBlank timing | repeat.
    m.write32(
        DMA0_CNT,
        1 | ((DMA_ENABLE | DMA_WORD | DMA_TIMING_VBLANK | DMA_REPEAT) << 16),
    );
    // Nothing happens until VBlank.
    m.tick(1000);
    assert_eq!(m.read32(EWRAM + 0x100), 0);
    // Advance to VBlank (line 160).
    m.tick(160 * 1232);
    assert_eq!(m.read32(EWRAM + 0x100), 0x1234_5678);
    // Repeat keeps the channel enabled for the next frame.
    assert_ne!(m.read16(0x0400_00BA) & 0x8000, 0);
}

// ------------------------------------------------------------------ timers

#[test]
fn timer_counts_up() {
    let mut m = blank();
    m.write16(TM0, 0); // reload 0
    m.write16(TM0 + 2, 0x80); // enable, prescaler 1
    m.tick(100);
    assert_eq!(m.read16(TM0), 100);
}

#[test]
fn timer_prescaler_divides() {
    let mut m = blank();
    m.write16(TM0, 0);
    m.write16(TM0 + 2, 0x81); // enable, prescaler /64
    m.tick(128);
    assert_eq!(m.read16(TM0), 2);
}

#[test]
fn timer_overflow_reloads_and_interrupts() {
    let mut m = blank();
    m.write16(TM0, 0xFFFE); // reload near the top
    m.write16(TM0 + 2, 0xC0); // enable, IRQ on overflow, prescaler 1
    m.tick(3); // FFFE -> FFFF -> overflow(reload FFFE) -> FFFF
    assert_eq!(m.read16(TM0), 0xFFFF);
    assert_eq!(m.read16(IF) & irq::TIMER0, irq::TIMER0);
}

#[test]
fn timer_cascade_counts_lower_overflows() {
    let mut m = blank();
    // Timer 0 overflows on every tick (reload = 0xFFFF).
    m.write16(TM0, 0xFFFF);
    m.write16(TM0 + 2, 0x80); // enable, prescaler 1, no IRQ
                              // Timer 1 in cascade mode counts timer 0's overflows.
    m.write16(TM1, 0);
    m.write16(TM1 + 2, 0x84); // enable, cascade
    m.tick(3);
    assert_eq!(m.read16(TM1), 3);
}

#[test]
fn timer_enable_edge_loads_reload() {
    let mut m = blank();
    m.write16(TM0, 0x1000);
    m.write16(TM0 + 2, 0x80); // enabling loads the counter from reload
    assert_eq!(m.read16(TM0), 0x1000);
}

// ------------------------------------------------------- CPU integration

#[test]
fn gba_stepping_drives_timers() {
    // A spinning ROM; the timer must advance purely from CPU stepping, which
    // ticks the peripheral clock each instruction.
    let mut rom = Vec::new();
    rom.extend_from_slice(&0xEAFF_FFFEu32.to_le_bytes()); // b .
    let mut gba = Gba::new(rom).unwrap();
    gba.mem.write16(TM0, 0);
    gba.mem.write16(TM0 + 2, 0x80); // enable, prescaler 1
    for _ in 0..1000 {
        gba.step();
    }
    assert!(gba.mem.read16(TM0) > 0, "timer advanced while the CPU ran");
}
