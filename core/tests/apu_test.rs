//! Phase 7 (part A) tests: the four PSG channels, mixing, and the sample
//! stream — verified numerically since there is no audio output yet.

use gba_core::apu::SAMPLE_RATE;
use gba_core::memory::Bus;
use gba_core::Memory;

// Sound registers.
const SND1_H: u32 = 0x0400_0062;
const SND1_X: u32 = 0x0400_0064;
const SND3_L: u32 = 0x0400_0070;
const SND3_H: u32 = 0x0400_0072;
const SND3_X: u32 = 0x0400_0074;
const WAVE_RAM: u32 = 0x0400_0090;
const SOUNDCNT_L: u32 = 0x0400_0080;
const SOUNDCNT_H: u32 = 0x0400_0082;
const SOUNDCNT_X: u32 = 0x0400_0084;

fn apu() -> Memory {
    let mut m = Memory::new(vec![0; 0x100]).unwrap();
    m.write16(SOUNDCNT_X, 0x80); // master enable
    m.write16(SOUNDCNT_H, 0x0002); // PSG volume 100%
    m
}

#[test]
fn sample_buffer_fills_at_the_output_rate() {
    let mut m = apu();
    m.tick(16_777_216); // one second of cycles
    let frames = m.drain_samples().len() / 2;
    assert!(
        (frames as i64 - SAMPLE_RATE as i64).abs() <= 2,
        "≈{SAMPLE_RATE} stereo frames per second, got {frames}"
    );
}

#[test]
fn square_channel_produces_an_oscillating_tone() {
    let mut m = apu();
    // Enable channel 1 on both sides at full master volume.
    m.write16(SOUNDCNT_L, 0x1177);
    // Duty 50%, full initial volume, DAC on.
    m.write16(SND1_H, 0xF080);
    // ~1 kHz square (2048 - 1920 = 128), trigger.
    m.write16(SND1_X, 0x8780);

    m.tick(200_000);
    let samples = m.drain_samples();
    let max = samples.iter().copied().max().unwrap();
    assert!(max > 1000, "tone should have loud peaks, got {max}");
    assert!(samples.contains(&0), "and silent troughs (50% duty)");
}

#[test]
fn master_disable_silences_output() {
    let mut m = apu();
    m.write16(SOUNDCNT_L, 0x1177);
    m.write16(SND1_H, 0xF080);
    m.write16(SND1_X, 0x8780);
    m.write16(SOUNDCNT_X, 0x00); // master disable
    m.tick(50_000);
    assert!(
        m.drain_samples().iter().all(|&s| s == 0),
        "no sound when disabled"
    );
}

#[test]
fn length_counter_disables_channel() {
    let mut m = apu();
    m.write16(SOUNDCNT_L, 0x1177);
    // Length load 63 → 1 tick of length; DAC on; enable length + trigger.
    m.write16(SND1_H, 0xF0BF);
    m.write16(SND1_X, 0xC780); // length enable (bit 14) + trigger
    assert_eq!(
        m.read16(SOUNDCNT_X) & 1,
        1,
        "channel 1 active right after trigger"
    );
    // The 256 Hz length clock reaches the counter within a few frames.
    m.tick(16_777_216 / 32);
    assert_eq!(
        m.read16(SOUNDCNT_X) & 1,
        0,
        "channel 1 turned off by length"
    );
}

#[test]
fn envelope_decreases_volume_over_time() {
    let mut m = apu();
    m.write16(SOUNDCNT_L, 0x1177);
    // Full volume, decreasing envelope with a short period (1), DAC on.
    m.write16(SND1_H, 0xF180);
    m.write16(SND1_X, 0x8780);
    m.tick(100_000);
    let early = m.drain_samples().iter().copied().max().unwrap();
    // Let the envelope run down for a while.
    m.tick(16_777_216 / 2);
    m.drain_samples();
    m.tick(100_000);
    let late = m.drain_samples().iter().copied().max().unwrap();
    assert!(
        late < early,
        "peak amplitude falls as the envelope decays ({early} -> {late})"
    );
}

const FIFO_A: u32 = 0x0400_00A0;
const TM0: u32 = 0x0400_0100;
const DMA1_SAD: u32 = 0x0400_00BC;
const DMA1_DAD: u32 = 0x0400_00C0;
const DMA1_CNT: u32 = 0x0400_00C4;
const EWRAM: u32 = 0x0200_0000;

#[test]
fn direct_sound_fifo_clocked_by_timer() {
    let mut m = apu();
    // Enable Direct Sound A on both sides at 100%, clocked by timer 0.
    m.write16(SOUNDCNT_H, 0x0304);
    // Fill the 32-byte FIFO with samples of value 100.
    for _ in 0..8 {
        m.write32(FIFO_A, 0x6464_6464);
    }
    // Timer 0 overflows every 256 cycles.
    m.write16(TM0, 0xFF00);
    m.write16(TM0 + 2, 0x80);
    for _ in 0..30 {
        m.tick(256);
    }
    let samples = m.drain_samples();
    assert!(
        samples.iter().any(|&s| s > 1000),
        "the popped PCM sample reaches the mix"
    );
}

#[test]
fn direct_sound_fifo_refilled_by_dma() {
    let mut m = apu();
    m.write16(SOUNDCNT_H, 0x0304); // DSA on, timer 0
                                   // Sample data in EWRAM.
    for i in 0..64 {
        m.write32(EWRAM + i * 4, 0x6464_6464);
    }
    // DMA1: EWRAM -> FIFO_A, 32-bit, dest fixed, repeat, special (FIFO) timing.
    m.write32(DMA1_SAD, EWRAM);
    m.write32(DMA1_DAD, FIFO_A);
    let ctrl: u32 = (1 << 15) | (1 << 10) | (2 << 5) | (1 << 9) | (3 << 12);
    m.write32(DMA1_CNT, 4 | (ctrl << 16));
    // Timer 0 overflows every 256 cycles.
    m.write16(TM0, 0xFF00);
    m.write16(TM0 + 2, 0x80);

    // Pop far more samples than the FIFO holds; DMA must keep refilling it.
    for _ in 0..200 {
        m.tick(256);
    }
    let samples = m.drain_samples();
    let tail = &samples[samples.len() - 20..];
    assert!(
        tail.iter().any(|&s| s > 1000),
        "DMA keeps the FIFO fed, so sound continues past 32 samples"
    );
}

#[test]
fn wave_channel_outputs_wave_ram() {
    let mut m = apu();
    m.write16(SOUNDCNT_L, 0x4444); // enable channel 3 (bit 2 / bit 6 groups)
                                   // Fill wave RAM with a ramp.
    for i in 0..8 {
        m.write16(WAVE_RAM + i * 2, 0x1032 + (i as u16) * 0x0404);
    }
    m.write16(SND3_L, 0x0080); // DAC on
    m.write16(SND3_H, 0x2000); // volume 100%
    m.write16(SND3_X, 0x8700); // freq, trigger
    m.tick(200_000);
    let samples = m.drain_samples();
    assert!(
        samples.iter().any(|&s| s != 0),
        "wave channel produces output"
    );
}
