//! Audio Processing Unit — the four PSG (Game Boy) channels plus Direct Sound.
//!
//! * Channel 1 — square wave with frequency sweep and a volume envelope.
//! * Channel 2 — square wave with a volume envelope.
//! * Channel 3 — wave output from 32 nibbles of wave RAM.
//! * Channel 4 — noise from a 15/7-bit LFSR with a volume envelope.
//!
//! A 512 Hz frame sequencer clocks the length counters (256 Hz), volume
//! envelopes (64 Hz) and the sweep (128 Hz). The channels are mixed to stereo
//! per `SOUNDCNT_L`/`SOUNDCNT_H` and resampled to [`SAMPLE_RATE`] into an
//! output buffer the frontend will later drain into `cpal`.
//!
//! Direct Sound: two 8-bit PCM FIFOs (A and B), each clocked by a timer
//! overflow and refilled by DMA when half-empty. This is what most commercial
//! games use for music.

use std::collections::VecDeque;

/// Output sample rate (stereo). 16.78 MHz / 512 = 32768 Hz.
pub const SAMPLE_RATE: u32 = 32768;
const CYCLES_PER_SAMPLE: u32 = 16_777_216 / SAMPLE_RATE;
/// The frame sequencer ticks at 512 Hz.
const FRAME_SEQ_PERIOD: u32 = 16_777_216 / 512;

const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];

/// A square-wave channel (used by channels 1 and 2; only channel 1 sweeps).
#[derive(Default)]
struct Square {
    // Registers.
    duty: u8,
    freq: u16, // 11-bit
    length_load: u8,
    env_initial: u8,
    env_add: bool,
    env_period: u8,
    length_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    has_sweep: bool,
    // Live state.
    enabled: bool,
    dac_on: bool,
    timer: i32,
    duty_pos: u8,
    length: u16,
    volume: u8,
    env_timer: u8,
    sweep_timer: u8,
    sweep_on: bool,
    sweep_shadow: u16,
}

impl Square {
    fn new(has_sweep: bool) -> Self {
        Square {
            has_sweep,
            ..Default::default()
        }
    }

    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 16
    }

    fn tick(&mut self, cycles: i32) {
        if !self.enabled {
            return;
        }
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period().max(1);
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_on;
        self.timer = self.period();
        if self.length == 0 {
            self.length = 64;
        }
        self.volume = self.env_initial;
        self.env_timer = self.env_period;
        if self.has_sweep {
            self.sweep_shadow = self.freq;
            self.sweep_timer = if self.sweep_period == 0 {
                8
            } else {
                self.sweep_period
            };
            self.sweep_on = self.sweep_period > 0 || self.sweep_shift > 0;
            if self.sweep_shift > 0 {
                self.sweep_calc(); // an immediate overflow check can disable the channel
            }
        }
    }

    fn clock_length(&mut self) {
        if self.length_enabled && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn clock_envelope(&mut self) {
        if self.env_period == 0 {
            return;
        }
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_add && self.volume < 15 {
                self.volume += 1;
            } else if !self.env_add && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }

    /// One sweep calculation; returns the new frequency and disables the
    /// channel on overflow.
    fn sweep_calc(&mut self) -> u16 {
        let delta = self.sweep_shadow >> self.sweep_shift;
        let new = if self.sweep_negate {
            self.sweep_shadow.wrapping_sub(delta)
        } else {
            self.sweep_shadow + delta
        };
        if new > 2047 {
            self.enabled = false;
        }
        new
    }

    fn clock_sweep(&mut self) {
        if !self.has_sweep || !self.sweep_on {
            return;
        }
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            self.sweep_timer = if self.sweep_period == 0 {
                8
            } else {
                self.sweep_period
            };
            if self.sweep_period > 0 {
                let new = self.sweep_calc();
                if new <= 2047 && self.sweep_shift > 0 {
                    self.sweep_shadow = new;
                    self.freq = new;
                    self.sweep_calc(); // second overflow check
                }
            }
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || !self.dac_on {
            return 0;
        }
        if (DUTY[self.duty as usize] >> self.duty_pos) & 1 != 0 {
            self.volume
        } else {
            0
        }
    }
}

/// Wave channel (channel 3): 32 4-bit samples from wave RAM.
#[derive(Default)]
struct Wave {
    freq: u16,
    length_load: u8,
    length_enabled: bool,
    volume_shift: u8, // 0 = mute, 1 = 100%, 2 = 50%, 3 = 25%
    dac_on: bool,
    enabled: bool,
    timer: i32,
    position: usize,
    length: u16,
    ram: [u8; 16], // 32 nibbles
}

impl Wave {
    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 8
    }

    fn tick(&mut self, cycles: i32) {
        if !self.enabled {
            return;
        }
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period().max(1);
            self.position = (self.position + 1) & 31;
        }
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_on;
        self.timer = self.period();
        if self.length == 0 {
            self.length = 256;
        }
        self.position = 0;
    }

    fn clock_length(&mut self) {
        if self.length_enabled && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || !self.dac_on || self.volume_shift == 0 {
            return 0;
        }
        let byte = self.ram[self.position / 2];
        let sample = if self.position & 1 == 0 {
            byte >> 4
        } else {
            byte & 0xF
        };
        sample >> (self.volume_shift - 1)
    }
}

/// Noise channel (channel 4): LFSR-driven pseudo-random output.
#[derive(Default)]
struct Noise {
    length_load: u8,
    length_enabled: bool,
    env_initial: u8,
    env_add: bool,
    env_period: u8,
    divisor_code: u8,
    width_7bit: bool,
    shift: u8,
    dac_on: bool,
    enabled: bool,
    timer: i32,
    lfsr: u16,
    length: u16,
    volume: u8,
    env_timer: u8,
}

impl Noise {
    fn period(&self) -> i32 {
        let divisor = if self.divisor_code == 0 {
            8
        } else {
            self.divisor_code as i32 * 16
        };
        divisor * (1 << (self.shift + 1)) * 2
    }

    fn tick(&mut self, cycles: i32) {
        if !self.enabled || self.shift >= 14 {
            return;
        }
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period().max(1);
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.width_7bit {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        }
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_on;
        self.timer = self.period();
        if self.length == 0 {
            self.length = 64;
        }
        self.volume = self.env_initial;
        self.env_timer = self.env_period;
        self.lfsr = 0x7FFF;
    }

    fn clock_length(&mut self) {
        if self.length_enabled && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn clock_envelope(&mut self) {
        if self.env_period == 0 {
            return;
        }
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_add && self.volume < 15 {
                self.volume += 1;
            } else if !self.env_add && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || !self.dac_on {
            return 0;
        }
        // Output is high when the low LFSR bit is 0.
        if self.lfsr & 1 == 0 {
            self.volume
        } else {
            0
        }
    }
}

pub struct Apu {
    ch1: Square,
    ch2: Square,
    ch3: Wave,
    ch4: Noise,
    /// SOUNDCNT_L: per-channel L/R enables and master volumes.
    cnt_l: u16,
    /// SOUNDCNT_H: PSG/Direct-Sound mixing.
    cnt_h: u16,
    /// SOUNDCNT_X: master enable (bit 7) + channel status (read-only 0-3).
    master_enable: bool,
    soundbias: u16,
    frame_seq_step: u8,
    frame_seq_timer: u32,
    sample_timer: u32,
    /// Interleaved stereo samples (L, R), i16.
    buffer: Vec<i16>,
    /// Direct Sound FIFOs (up to 32 bytes) and their current output samples.
    fifo_a: VecDeque<i8>,
    fifo_b: VecDeque<i8>,
    dsa_out: i8,
    dsb_out: i8,
}

impl Apu {
    pub fn new() -> Self {
        Apu {
            ch1: Square::new(true),
            ch2: Square::new(false),
            ch3: Wave::default(),
            ch4: Noise::default(),
            cnt_l: 0,
            cnt_h: 0,
            master_enable: false,
            soundbias: 0x200,
            frame_seq_step: 0,
            frame_seq_timer: FRAME_SEQ_PERIOD,
            sample_timer: CYCLES_PER_SAMPLE,
            buffer: Vec::new(),
            fifo_a: VecDeque::with_capacity(32),
            fifo_b: VecDeque::with_capacity(32),
            dsa_out: 0,
            dsb_out: 0,
        }
    }

    /// A timer (0 or 1) overflowed: clock any Direct Sound channel bound to it,
    /// popping the next PCM sample from its FIFO.
    pub fn on_timer_overflow(&mut self, timer: u8) {
        if u16::from(timer) == (self.cnt_h >> 10) & 1 {
            self.dsa_out = self.fifo_a.pop_front().unwrap_or(0);
        }
        if u16::from(timer) == (self.cnt_h >> 14) & 1 {
            self.dsb_out = self.fifo_b.pop_front().unwrap_or(0);
        }
    }

    /// Whether Direct Sound FIFO `ch` (0 = A, 1 = B) is at most half full and
    /// should be refilled by DMA.
    pub fn fifo_needs_dma(&self, ch: usize) -> bool {
        let len = if ch == 0 {
            self.fifo_a.len()
        } else {
            self.fifo_b.len()
        };
        len <= 16
    }

    /// Take the accumulated stereo samples, clearing the buffer.
    pub fn drain_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.buffer)
    }

    /// Number of stereo frames currently buffered.
    pub fn buffered_frames(&self) -> usize {
        self.buffer.len() / 2
    }

    pub fn tick(&mut self, cycles: u64) {
        // Process in chunks no larger than the next output-sample boundary, so
        // a single large tick still produces every sample (and keeps the
        // waveform advancing between them).
        let mut remaining = cycles;
        while remaining > 0 {
            let step = remaining.min(u64::from(self.sample_timer));
            let s = step as i32;
            if self.master_enable {
                self.ch1.tick(s);
                self.ch2.tick(s);
                self.ch3.tick(s);
                self.ch4.tick(s);
            }

            // Frame sequencer at 512 Hz (period far exceeds a sample chunk, so
            // at most one step fires per chunk).
            if step >= u64::from(self.frame_seq_timer) {
                let over = (step - u64::from(self.frame_seq_timer)) as u32;
                self.frame_seq_timer = FRAME_SEQ_PERIOD - over % FRAME_SEQ_PERIOD;
                self.clock_frame_sequencer();
            } else {
                self.frame_seq_timer -= step as u32;
            }

            self.sample_timer -= step as u32;
            if self.sample_timer == 0 {
                self.sample_timer = CYCLES_PER_SAMPLE;
                self.mix_sample();
            }
            remaining -= step;
        }
    }

    fn clock_frame_sequencer(&mut self) {
        // Steps 0,2,4,6 clock length; 2,6 clock sweep; 7 clocks envelope.
        if self.frame_seq_step.is_multiple_of(2) {
            self.ch1.clock_length();
            self.ch2.clock_length();
            self.ch3.clock_length();
            self.ch4.clock_length();
        }
        if self.frame_seq_step == 2 || self.frame_seq_step == 6 {
            self.ch1.clock_sweep();
        }
        if self.frame_seq_step == 7 {
            self.ch1.clock_envelope();
            self.ch2.clock_envelope();
            self.ch4.clock_envelope();
        }
        self.frame_seq_step = (self.frame_seq_step + 1) & 7;
    }

    fn mix_sample(&mut self) {
        if !self.master_enable {
            self.buffer.push(0);
            self.buffer.push(0);
            return;
        }
        let out = [
            self.ch1.output(),
            self.ch2.output(),
            self.ch3.output(),
            self.ch4.output(),
        ];
        // PSG master volume from SOUNDCNT_H bits 0-1 (0=25%, 1=50%, 2=100%).
        let psg_vol = match self.cnt_h & 0x3 {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        // SOUNDCNT_L: bits 8-11 enable channels on the right, 12-15 on the left.
        let mut left = 0i32;
        let mut right = 0i32;
        for (i, &o) in out.iter().enumerate() {
            if self.cnt_l & (1 << (12 + i)) != 0 {
                left += o as i32;
            }
            if self.cnt_l & (1 << (8 + i)) != 0 {
                right += o as i32;
            }
        }
        // Per-side master volume (SOUNDCNT_L bits 4-6 left, 0-2 right).
        let left_vol = ((self.cnt_l >> 4) & 0x7) as i32 + 1;
        let right_vol = (self.cnt_l & 0x7) as i32 + 1;
        let mut left = left * psg_vol * left_vol * 8;
        let mut right = right * psg_vol * right_vol * 8;

        // Direct Sound A/B are mixed in independently of SOUNDCNT_L, at 50% or
        // 100% volume (SOUNDCNT_H bits 2/3), enabled per side by bits 8/9 (A)
        // and 12/13 (B).
        let dsa = i32::from(self.dsa_out) * if self.cnt_h & 1 << 2 != 0 { 4 } else { 2 } * 8;
        let dsb = i32::from(self.dsb_out) * if self.cnt_h & 1 << 3 != 0 { 4 } else { 2 } * 8;
        if self.cnt_h & 1 << 9 != 0 {
            left += dsa;
        }
        if self.cnt_h & 1 << 8 != 0 {
            right += dsa;
        }
        if self.cnt_h & 1 << 13 != 0 {
            left += dsb;
        }
        if self.cnt_h & 1 << 12 != 0 {
            right += dsb;
        }

        self.buffer.push(left.clamp(-32768, 32767) as i16);
        self.buffer.push(right.clamp(-32768, 32767) as i16);
    }

    // ---- register access (0x060..0x0A8, plus wave RAM 0x090..0x0A0) ----

    pub fn read8(&self, offset: u32) -> u8 {
        let half = self.read16(offset & !1);
        if offset & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn read16(&self, offset: u32) -> u16 {
        match offset {
            0x60 => {
                u16::from(self.ch1.sweep_shift)
                    | u16::from(self.ch1.sweep_negate) << 3
                    | u16::from(self.ch1.sweep_period) << 4
            }
            0x62 => square_cnt_h(&self.ch1),
            0x68 => square_cnt_h(&self.ch2),
            0x70 => u16::from(self.ch3.dac_on) << 7,
            0x72 => u16::from(self.ch3.volume_shift) << 13,
            0x78 => 0,
            0x7C => noise_cnt_l(&self.ch4),
            0x80 => self.cnt_l,
            0x82 => self.cnt_h,
            0x84 => {
                u16::from(self.master_enable) << 7
                    | u16::from(self.ch1.enabled)
                    | u16::from(self.ch2.enabled) << 1
                    | u16::from(self.ch3.enabled) << 2
                    | u16::from(self.ch4.enabled) << 3
            }
            0x88 => self.soundbias,
            0x90..=0x9F => {
                let i = (offset - 0x90) as usize;
                u16::from(self.ch3.ram[i]) | u16::from(self.ch3.ram[i + 1]) << 8
            }
            _ => 0,
        }
    }

    pub fn write8(&mut self, offset: u32, value: u8) {
        // Wave RAM is byte-addressable; other registers compose via read16.
        if (0x90..0xA0).contains(&offset) {
            self.ch3.ram[(offset - 0x90) as usize] = value;
            return;
        }
        let half = self.read16(offset & !1);
        let merged = if offset & 1 == 0 {
            (half & 0xFF00) | u16::from(value)
        } else {
            (half & 0x00FF) | (u16::from(value) << 8)
        };
        self.write16(offset & !1, merged);
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        // Only SOUNDCNT_X's master-enable is writable while sound is off; the
        // GBA otherwise ignores writes to the channel registers when disabled.
        match offset {
            0x60 => {
                self.ch1.sweep_shift = (value & 0x7) as u8;
                self.ch1.sweep_negate = value & 0x8 != 0;
                self.ch1.sweep_period = ((value >> 4) & 0x7) as u8;
            }
            0x62 => write_square_cnt_h(&mut self.ch1, value),
            0x64 => write_square_cnt_x(&mut self.ch1, value),
            0x68 => write_square_cnt_h(&mut self.ch2, value),
            0x6C => write_square_cnt_x(&mut self.ch2, value),
            0x70 => {
                self.ch3.dac_on = value & 0x80 != 0;
                if !self.ch3.dac_on {
                    self.ch3.enabled = false;
                }
            }
            0x72 => {
                self.ch3.length_load = value as u8;
                self.ch3.length = 256 - u16::from(value as u8);
                self.ch3.volume_shift = ((value >> 13) & 0x3) as u8;
            }
            0x74 => {
                self.ch3.freq = value & 0x7FF;
                self.ch3.length_enabled = value & 0x4000 != 0;
                if value & 0x8000 != 0 {
                    self.ch3.trigger();
                }
            }
            0x78 => {
                self.ch4.length_load = (value & 0x3F) as u8;
                self.ch4.length = 64 - u16::from((value & 0x3F) as u8);
                self.ch4.env_period = ((value >> 8) & 0x7) as u8;
                self.ch4.env_add = value & 0x800 != 0;
                self.ch4.env_initial = ((value >> 12) & 0xF) as u8;
                self.ch4.dac_on = value & 0xF800 != 0;
                if !self.ch4.dac_on {
                    self.ch4.enabled = false;
                }
            }
            0x7C => {
                self.ch4.divisor_code = (value & 0x7) as u8;
                self.ch4.width_7bit = value & 0x8 != 0;
                self.ch4.shift = ((value >> 4) & 0xF) as u8;
                self.ch4.length_enabled = value & 0x4000 != 0;
                if value & 0x8000 != 0 {
                    self.ch4.trigger();
                }
            }
            0x80 => self.cnt_l = value,
            0x82 => {
                self.cnt_h = value;
                if value & 1 << 11 != 0 {
                    self.fifo_a.clear();
                    self.dsa_out = 0;
                }
                if value & 1 << 15 != 0 {
                    self.fifo_b.clear();
                    self.dsb_out = 0;
                }
            }
            // FIFO_A (0xA0) / FIFO_B (0xA4): 8-bit PCM samples, 32 bytes deep.
            0xA0 | 0xA2 => push_fifo(&mut self.fifo_a, value),
            0xA4 | 0xA6 => push_fifo(&mut self.fifo_b, value),
            0x84 => {
                self.master_enable = value & 0x80 != 0;
                if !self.master_enable {
                    self.silence();
                }
            }
            0x88 => self.soundbias = value,
            0x90..=0x9F => {
                let i = (offset - 0x90) as usize;
                self.ch3.ram[i] = value as u8;
                self.ch3.ram[i + 1] = (value >> 8) as u8;
            }
            _ => {}
        }
    }

    pub fn read32(&self, offset: u32) -> u32 {
        u32::from(self.read16(offset)) | u32::from(self.read16(offset + 2)) << 16
    }

    pub fn write32(&mut self, offset: u32, value: u32) {
        self.write16(offset, value as u16);
        self.write16(offset + 2, (value >> 16) as u16);
    }

    fn silence(&mut self) {
        self.ch1.enabled = false;
        self.ch2.enabled = false;
        self.ch3.enabled = false;
        self.ch4.enabled = false;
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

fn square_cnt_h(ch: &Square) -> u16 {
    u16::from(ch.duty) << 6
        | u16::from(ch.env_period)
        | u16::from(ch.env_add) << 3
        | u16::from(ch.env_initial) << 4
}

fn write_square_cnt_h(ch: &mut Square, value: u16) {
    ch.length_load = (value & 0x3F) as u8;
    ch.length = 64 - u16::from((value & 0x3F) as u8);
    ch.duty = ((value >> 6) & 0x3) as u8;
    ch.env_period = ((value >> 8) & 0x7) as u8;
    ch.env_add = value & 0x800 != 0;
    ch.env_initial = ((value >> 12) & 0xF) as u8;
    ch.dac_on = value & 0xF800 != 0;
    if !ch.dac_on {
        ch.enabled = false;
    }
}

fn write_square_cnt_x(ch: &mut Square, value: u16) {
    ch.freq = value & 0x7FF;
    ch.length_enabled = value & 0x4000 != 0;
    if value & 0x8000 != 0 {
        ch.trigger();
    }
}

fn noise_cnt_l(ch: &Noise) -> u16 {
    u16::from(ch.env_period) | u16::from(ch.env_add) << 3 | u16::from(ch.env_initial) << 4
}

/// Push a halfword's two bytes onto a Direct Sound FIFO (as signed samples),
/// dropping them if the 32-byte FIFO is already full.
fn push_fifo(fifo: &mut VecDeque<i8>, value: u16) {
    for byte in [value as u8, (value >> 8) as u8] {
        if fifo.len() < 32 {
            fifo.push_back(byte as i8);
        }
    }
}
