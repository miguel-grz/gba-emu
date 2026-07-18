//! DMA controller — four channels (DMA0–DMA3).
//!
//! This module owns the DMA *register state* and the latch/repeat bookkeeping;
//! the actual memory-to-memory copy is performed by
//! [`Memory`](crate::memory::Memory), which is the only place with access to
//! the whole address space. [`Memory::run_dma`](crate::memory::Memory) drives
//! the transfers on the right trigger (immediate, VBlank, or HBlank).
//!
//! Start-timing "special" (sound FIFO on DMA1/2, video capture on DMA3) is
//! deferred to Phase 7; those channels simply do not trigger yet. DMA does not
//! yet steal CPU cycles — a transfer is performed atomically between
//! instructions, which is accurate to the CPU's view of memory if not to
//! exact bus timing.

/// Control-register bit layout (CNT_H).
pub(crate) mod cnt {
    pub const DEST_CTL: u16 = 5; // bits 5–6
    pub const SRC_CTL: u16 = 7; // bits 7–8
    pub const REPEAT: u16 = 1 << 9;
    pub const WORD: u16 = 1 << 10; // 0 = 16-bit, 1 = 32-bit
    pub const TIMING: u16 = 12; // bits 12–13
    pub const IRQ: u16 = 1 << 14;
    pub const ENABLE: u16 = 1 << 15;
}

/// Address-adjustment mode for source/destination. Mode 0 (increment) is the
/// default handled by the caller, so it needs no named constant here.
pub(crate) const ADDR_DEC: u16 = 1;
pub(crate) const ADDR_FIXED: u16 = 2;
pub(crate) const ADDR_INC_RELOAD: u16 = 3;

#[derive(Default)]
pub(crate) struct DmaChannel {
    pub sad: u32,       // source register (raw)
    pub dad: u32,       // destination register (raw)
    pub count: u16,     // word-count register (raw)
    pub control: u16,   // CNT_H
    pub src: u32,       // internal latched source
    pub dst: u32,       // internal latched destination
    pub remaining: u32, // internal remaining units
    pub pending: bool,  // an immediate transfer is queued
}

pub struct Dma {
    pub(crate) ch: [DmaChannel; 4],
}

/// Source is 27-bit for DMA0–2 (internal memory only) and 28-bit for DMA3
/// (which can also read the cartridge).
pub(crate) fn src_mask(ch: usize) -> u32 {
    if ch == 3 {
        0x0FFF_FFFF
    } else {
        0x07FF_FFFF
    }
}

pub(crate) fn dst_mask(ch: usize) -> u32 {
    if ch == 3 {
        0x0FFF_FFFF
    } else {
        0x07FF_FFFF
    }
}

/// Maximum unit count (a `count` field of 0 means "transfer the maximum").
pub(crate) fn count_max(ch: usize) -> u32 {
    if ch == 3 {
        0x1_0000
    } else {
        0x4000
    }
}

fn count_mask(ch: usize) -> u32 {
    count_max(ch) - 1
}

impl Dma {
    pub fn new() -> Self {
        Dma {
            ch: Default::default(),
        }
    }

    /// Latch source/dest/count into the internal counters when a channel is
    /// enabled, and queue an immediate transfer if so configured.
    fn latch(&mut self, i: usize) {
        let ch = &mut self.ch[i];
        ch.src = ch.sad & src_mask(i);
        ch.dst = ch.dad & dst_mask(i);
        let c = u32::from(ch.count) & count_mask(i);
        ch.remaining = if c == 0 { count_max(i) } else { c };
        if (ch.control >> cnt::TIMING) & 0x3 == 0 {
            ch.pending = true; // immediate timing
        }
    }

    // ---- register access (0x0B0..0x0E0) ----

    pub fn read16(&self, offset: u32) -> u16 {
        let rel = (offset - 0xB0) as usize;
        let ch = rel / 12;
        if ch >= 4 {
            return 0;
        }
        // Only CNT_H is meaningfully readable; the rest are write-only.
        match rel % 12 {
            10 => self.ch[ch].control,
            _ => 0,
        }
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        let rel = (offset - 0xB0) as usize;
        let ch = rel / 12;
        if ch >= 4 {
            return;
        }
        match rel % 12 {
            0 => self.ch[ch].sad = (self.ch[ch].sad & 0xFFFF_0000) | u32::from(value),
            2 => self.ch[ch].sad = (self.ch[ch].sad & 0x0000_FFFF) | (u32::from(value) << 16),
            4 => self.ch[ch].dad = (self.ch[ch].dad & 0xFFFF_0000) | u32::from(value),
            6 => self.ch[ch].dad = (self.ch[ch].dad & 0x0000_FFFF) | (u32::from(value) << 16),
            8 => self.ch[ch].count = value,
            10 => {
                let was_enabled = self.ch[ch].control & cnt::ENABLE != 0;
                self.ch[ch].control = value;
                if value & cnt::ENABLE != 0 && !was_enabled {
                    self.latch(ch);
                }
            }
            _ => {}
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

impl Default for Dma {
    fn default() -> Self {
        Self::new()
    }
}
