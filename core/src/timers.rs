//! Hardware timers — four channels (TM0–TM3).
//!
//! Each timer counts up at a prescaled rate (1/64/256/1024 cycles) or, in
//! cascade (count-up) mode, once each time the timer below it overflows. On
//! overflow the counter reloads from its reload value and can raise an
//! interrupt. [`Timers::tick`] advances all four for a batch of cycles and
//! returns the interrupt bits to raise.
//!
//! Timing is cycle-accurate at the granularity of a `tick` call (the prescaler
//! remainder is carried between calls). Since a `tick` follows each CPU
//! instruction, that granularity is a few cycles — the sub-instruction phase
//! at which a timer IRQ lands is refined only if a later phase needs it.

use crate::io::irq;

const PRESCALER: [u32; 4] = [1, 64, 256, 1024];

/// Control-register bits (TMxCNT_H).
mod cnt {
    pub const CASCADE: u16 = 1 << 2;
    pub const IRQ: u16 = 1 << 6;
    pub const ENABLE: u16 = 1 << 7;
}

#[derive(Default)]
struct Timer {
    reload: u16,
    control: u16,
    counter: u16,
    /// Accumulated (un-prescaled) cycles not yet turned into an increment.
    prescaler_acc: u32,
}

impl Timer {
    fn enabled(&self) -> bool {
        self.control & cnt::ENABLE != 0
    }
    fn cascade(&self) -> bool {
        self.control & cnt::CASCADE != 0
    }
    fn irq_enabled(&self) -> bool {
        self.control & cnt::IRQ != 0
    }

    /// Apply `increments` counts, returning how many times it overflowed.
    fn add(&mut self, increments: u32) -> u32 {
        if increments == 0 {
            return 0;
        }
        let space = 0x1_0000 - u32::from(self.counter);
        if increments < space {
            self.counter += increments as u16;
            0
        } else {
            let period = 0x1_0000 - u32::from(self.reload);
            let past = increments - space;
            let overflows = 1 + past / period;
            self.counter = self.reload.wrapping_add((past % period) as u16);
            overflows
        }
    }
}

pub struct Timers {
    timers: [Timer; 4],
}

impl Timers {
    pub fn new() -> Self {
        Timers {
            timers: Default::default(),
        }
    }

    /// Advance all timers by `cycles`. Returns the interrupt bits to raise and
    /// a mask of which of timers 0/1 overflowed (bit 0 / bit 1), used to clock
    /// Direct Sound. Processed low-to-high so a cascade timer sees this tick's
    /// overflows from the timer below it.
    pub fn tick(&mut self, cycles: u64) -> (u16, u8) {
        let mut irqs = 0;
        let mut sound_overflow = 0u8;
        let mut prev_overflows = 0u32;
        for i in 0..4 {
            let t = &mut self.timers[i];
            if !t.enabled() {
                prev_overflows = 0;
                continue;
            }
            let increments = if i > 0 && t.cascade() {
                prev_overflows
            } else {
                let pres = PRESCALER[(t.control & 0x3) as usize];
                t.prescaler_acc += cycles as u32;
                let inc = t.prescaler_acc / pres;
                t.prescaler_acc %= pres;
                inc
            };
            let overflows = t.add(increments);
            if overflows > 0 && t.irq_enabled() {
                irqs |= irq::TIMER0 << i;
            }
            if overflows > 0 && i < 2 {
                sound_overflow |= 1 << i;
            }
            prev_overflows = overflows;
        }
        (irqs, sound_overflow)
    }

    // ---- register access (0x100..0x110) ----

    pub fn read16(&self, offset: u32) -> u16 {
        let i = ((offset - 0x100) / 4) as usize;
        if i >= 4 {
            return 0;
        }
        if (offset - 0x100) % 4 < 2 {
            self.timers[i].counter // TMxCNT_L reads the live counter
        } else {
            self.timers[i].control
        }
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        let i = ((offset - 0x100) / 4) as usize;
        if i >= 4 {
            return;
        }
        if (offset - 0x100) % 4 < 2 {
            self.timers[i].reload = value; // TMxCNT_L writes the reload value
        } else {
            let was_enabled = self.timers[i].enabled();
            self.timers[i].control = value;
            // A disabled→enabled edge reloads the counter and resets the
            // prescaler phase.
            if value & cnt::ENABLE != 0 && !was_enabled {
                self.timers[i].counter = self.timers[i].reload;
                self.timers[i].prescaler_acc = 0;
            }
        }
    }

    pub fn read8(&self, offset: u32) -> u8 {
        let half = self.read16(offset & !1);
        if offset & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn write8(&mut self, offset: u32, value: u8) {
        let half = self.read16(offset & !1);
        let merged = if offset & 1 == 0 {
            (half & 0xFF00) | u16::from(value)
        } else {
            (half & 0x00FF) | (u16::from(value) << 8)
        };
        self.write16(offset & !1, merged);
    }

    pub fn read32(&self, offset: u32) -> u32 {
        u32::from(self.read16(offset)) | u32::from(self.read16(offset + 2)) << 16
    }

    pub fn write32(&mut self, offset: u32, value: u32) {
        self.write16(offset, value as u16);
        self.write16(offset + 2, (value >> 16) as u16);
    }
}

impl Default for Timers {
    fn default() -> Self {
        Self::new()
    }
}
