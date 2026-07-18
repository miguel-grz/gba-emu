//! GBA memory map and bus.
//!
//! Phase 2 turns the Phase-1 stub into a real bus: every region is decoded and
//! timed, the I/O register file is wired in (see [`crate::io`]), cartridge
//! open-bus reads are modeled, and the 8-bit-write quirks of palette/VRAM/OAM
//! are honored. Access timing (waitstates, S/N cycles) is accumulated per
//! access and handed to the CPU so its cycle counter reflects the real bus —
//! see [`crate::timing`] for the model and its documented approximations.
//!
//! Still stubbed for later phases: cartridge SRAM/Flash/EEPROM saves (reads
//! return 0), and BIOS read-protection (the anti-tamper behavior where the
//! BIOS region reads as its last fetched opcode unless the CPU is executing
//! from it). Neither is needed to run CPU test ROMs.

use crate::apu::Apu;
use crate::dma::{self, cnt, Dma};
use crate::io::{irq, Io};
use crate::ppu::Ppu;
use crate::timers::Timers;
use crate::timing::{access_cycles, Region, SeqTracker, Width};
use crate::CoreError;
use std::path::Path;

/// I/O sub-block boundaries within 0x0400_0000. Offsets outside the PPU/sound/
/// DMA/timer ranges fall through to [`Io`] (interrupt controller, WAITCNT, …).
const PPU_REG_END: u32 = 0x60;
const SOUND_REG_START: u32 = 0x60;
const SOUND_REG_END: u32 = 0xA8;
const DMA_REG_START: u32 = 0xB0;
const DMA_REG_END: u32 = 0xE0;
const TIMER_REG_START: u32 = 0x100;
const TIMER_REG_END: u32 = 0x110;

pub const BIOS_SIZE: usize = 16 * 1024;
pub const EWRAM_SIZE: usize = 256 * 1024;
pub const IWRAM_SIZE: usize = 32 * 1024;
pub const PALETTE_SIZE: usize = 1024;
pub const VRAM_SIZE: usize = 96 * 1024;
pub const OAM_SIZE: usize = 1024;
pub const ROM_MAX_SIZE: usize = 32 * 1024 * 1024;

/// CPU-visible address space. The CPU is generic over this trait, so tests can
/// substitute a flat RAM bus and later phases can extend [`Memory`] without
/// touching the CPU.
///
/// Byte access is required; the 16/32-bit accessors have byte-composed
/// defaults for simple test buses. [`Memory`] overrides all of them so that
/// each access is a single timed, correctly-sized bus operation.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn write8(&mut self, addr: u32, value: u8);

    fn read16(&mut self, addr: u32) -> u16 {
        let a = addr & !1;
        u16::from(self.read8(a)) | u16::from(self.read8(a | 1)) << 8
    }

    fn read32(&mut self, addr: u32) -> u32 {
        let a = addr & !3;
        u32::from(self.read16(a)) | u32::from(self.read16(a | 2)) << 16
    }

    fn write16(&mut self, addr: u32, value: u16) {
        let a = addr & !1;
        self.write8(a, value as u8);
        self.write8(a | 1, (value >> 8) as u8);
    }

    fn write32(&mut self, addr: u32, value: u32) {
        let a = addr & !3;
        self.write16(a, value as u16);
        self.write16(a | 2, (value >> 16) as u16);
    }

    /// Whether a real BIOS image is present. `false` selects HLE SWI handling.
    fn has_bios(&self) -> bool {
        false
    }

    /// Whether an unmasked interrupt is pending (drives CPU IRQ dispatch).
    fn irq_pending(&self) -> bool {
        false
    }

    /// Take (and clear) a pending HALTCNT request from the I/O registers.
    fn take_halt_request(&mut self) -> bool {
        false
    }

    /// Take (and reset) the waitstate cycles accumulated since the last call.
    fn take_access_cycles(&mut self) -> u32 {
        0
    }
}

/// The GBA memory map.
pub struct Memory {
    bios: Vec<u8>,
    ewram: Vec<u8>,
    iwram: Vec<u8>,
    palette: Vec<u8>,
    vram: Vec<u8>,
    oam: Vec<u8>,
    rom: Vec<u8>,
    io: Io,
    ppu: Ppu,
    apu: Apu,
    dma: Dma,
    timers: Timers,
    has_bios: bool,
    /// Waitstate cycles accumulated since the CPU last drained them.
    access_cycles: u32,
    seq: SeqTracker,
    /// Last value driven on the bus, returned for open-bus reads.
    open_bus: u32,
}

impl Memory {
    /// Create a memory map with the given cartridge ROM and a zero-filled BIOS
    /// (HLE mode).
    pub fn new(rom: Vec<u8>) -> Result<Self, CoreError> {
        if rom.len() > ROM_MAX_SIZE {
            return Err(CoreError::RomTooLarge {
                size: rom.len(),
                max: ROM_MAX_SIZE,
            });
        }
        Ok(Self {
            bios: vec![0; BIOS_SIZE],
            ewram: vec![0; EWRAM_SIZE],
            iwram: vec![0; IWRAM_SIZE],
            palette: vec![0; PALETTE_SIZE],
            vram: vec![0; VRAM_SIZE],
            oam: vec![0; OAM_SIZE],
            rom,
            io: Io::new(),
            ppu: Ppu::new(),
            apu: Apu::new(),
            dma: Dma::new(),
            timers: Timers::new(),
            has_bios: false,
            access_cycles: 0,
            seq: SeqTracker::default(),
            open_bus: 0,
        })
    }

    /// Load a real BIOS image (exactly 16 KiB), switching to LLE mode.
    pub fn load_bios(&mut self, bios: Vec<u8>) -> Result<(), CoreError> {
        if bios.len() != BIOS_SIZE {
            return Err(CoreError::BadBiosSize {
                size: bios.len(),
                expected: BIOS_SIZE,
            });
        }
        self.bios = bios;
        self.has_bios = true;
        Ok(())
    }

    /// Load a ROM file, and optionally a BIOS file.
    pub fn from_files(rom: &Path, bios: Option<&Path>) -> Result<Self, CoreError> {
        let mut mem = Self::new(std::fs::read(rom)?)?;
        if let Some(b) = bios {
            mem.load_bios(std::fs::read(b)?)?;
        }
        Ok(mem)
    }

    /// Raise interrupt request bit(s) in `IF`. Peripherals use this in later
    /// phases; tests drive the interrupt path with it today.
    pub fn raise_irq(&mut self, bits: u16) {
        self.io.raise_irq(bits);
    }

    /// Advance the timers, PPU and DMA by `cycles`, raising any resulting
    /// interrupts. Call this after each `Cpu::step` with the cycles it
    /// reported.
    pub fn tick(&mut self, cycles: u64) {
        let timer_irqs = self.timers.tick(cycles);
        self.apu.tick(cycles);
        let out = self.ppu.tick(cycles, &self.vram, &self.palette, &self.oam);
        let irqs = timer_irqs | out.irqs;
        if irqs != 0 {
            self.io.raise_irq(irqs);
        }
        self.run_dma(out.vblank_start, out.hblank_start);
    }

    /// Take the accumulated stereo audio samples (interleaved L/R, i16).
    pub fn drain_samples(&mut self) -> Vec<i16> {
        self.apu.drain_samples()
    }

    /// Run any DMA channels triggered this step. Channels are serviced in
    /// priority order (0 highest), each transferring atomically.
    fn run_dma(&mut self, vblank: bool, hblank: bool) {
        for i in 0..4 {
            let ctrl = self.dma.ch[i].control;
            if ctrl & cnt::ENABLE == 0 {
                continue;
            }
            let timing = (ctrl >> cnt::TIMING) & 0x3;
            let triggered = match timing {
                0 => std::mem::take(&mut self.dma.ch[i].pending),
                1 => vblank,
                2 => hblank,
                _ => false, // special (sound FIFO / video) — Phase 7
            };
            if triggered {
                self.transfer_dma(i);
            }
        }
    }

    /// Perform one channel's transfer, then apply repeat/reload or disable and
    /// raise the completion interrupt.
    fn transfer_dma(&mut self, i: usize) {
        let ctrl = self.dma.ch[i].control;
        let word = ctrl & cnt::WORD != 0;
        let width = if word { Width::Word } else { Width::Half };
        let step: u32 = if word { 4 } else { 2 };
        let dst_ctl = (ctrl >> cnt::DEST_CTL) & 0x3;
        let src_ctl = (ctrl >> cnt::SRC_CTL) & 0x3;

        let mut src = self.dma.ch[i].src;
        let mut dst = self.dma.ch[i].dst;
        let count = self.dma.ch[i].remaining;

        let advance = |addr: u32, mode: u16| match mode {
            dma::ADDR_DEC => addr.wrapping_sub(step),
            dma::ADDR_FIXED => addr,
            _ => addr.wrapping_add(step), // inc / inc+reload
        };
        let align = if word { !3 } else { !1 };

        for _ in 0..count {
            let value = self.decode_read(src & align, width);
            self.decode_write(dst & align, width, value);
            src = advance(src, src_ctl);
            dst = advance(dst, dst_ctl);
        }
        self.dma.ch[i].src = src;

        let repeat = ctrl & cnt::REPEAT != 0;
        let timing = (ctrl >> cnt::TIMING) & 0x3;
        if repeat && timing != 0 {
            // Reload the count; reload the destination too if configured.
            let c = u32::from(self.dma.ch[i].count) & (dma::count_max(i) - 1);
            self.dma.ch[i].remaining = if c == 0 { dma::count_max(i) } else { c };
            if dst_ctl == dma::ADDR_INC_RELOAD {
                self.dma.ch[i].dst = self.dma.ch[i].dad & dma::dst_mask(i);
            } else {
                self.dma.ch[i].dst = dst;
            }
        } else {
            self.dma.ch[i].dst = dst;
            self.dma.ch[i].control &= !cnt::ENABLE;
        }

        if ctrl & cnt::IRQ != 0 {
            self.io.raise_irq(irq::DMA0 << i);
        }
    }

    /// The PPU's 15-bit BGR555 framebuffer (240×160).
    pub fn framebuffer(&self) -> &[u16] {
        self.ppu.framebuffer()
    }

    /// Take (and clear) the "a frame finished drawing" flag from the PPU.
    pub fn take_frame_ready(&mut self) -> bool {
        self.ppu.take_frame_ready()
    }

    /// Current scanline (VCOUNT).
    pub fn vcount(&self) -> u16 {
        self.ppu.vcount()
    }

    /// Read-only view of VRAM, for headless render checks.
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// VRAM is 96 KiB in a 128 KiB window: the upper 32 KiB mirrors 64K–96K.
    fn vram_index(addr: u32) -> usize {
        let a = (addr & 0x1_FFFF) as usize;
        if a >= 0x18000 {
            a - 0x8000
        } else {
            a
        }
    }

    fn width_bytes(width: Width) -> u32 {
        match width {
            Width::Byte => 1,
            Width::Half => 2,
            Width::Word => 4,
        }
    }

    /// Load `width` bytes (little-endian) from a RAM/ROM slice, wrapping the
    /// index modulo `len` so mirrored regions wrap instead of panicking.
    /// `len` need not be a power of two (VRAM is 96 KiB).
    fn load_slice(data: &[u8], len: usize, base: usize, width: Width) -> u32 {
        let mut value = 0u32;
        for i in 0..Self::width_bytes(width) as usize {
            value |= u32::from(data[(base + i) % len]) << (8 * i);
        }
        value
    }

    fn store_slice(data: &mut [u8], len: usize, base: usize, width: Width, value: u32) {
        for i in 0..Self::width_bytes(width) as usize {
            data[(base + i) % len] = (value >> (8 * i)) as u8;
        }
    }

    /// Decode and perform a read of the given width, without timing.
    fn decode_read(&mut self, addr: u32, width: Width) -> u32 {
        match addr >> 24 {
            0x00 | 0x01 => {
                if (addr as usize) < BIOS_SIZE {
                    Self::load_slice(&self.bios, BIOS_SIZE, addr as usize, width)
                } else {
                    self.open_bus
                }
            }
            0x02 => Self::load_slice(&self.ewram, EWRAM_SIZE, addr as usize, width),
            0x03 => Self::load_slice(&self.iwram, IWRAM_SIZE, addr as usize, width),
            0x04 => self.io_read(addr & 0x3FF, width),
            0x05 => Self::load_slice(&self.palette, PALETTE_SIZE, addr as usize, width),
            0x06 => {
                let idx = Self::vram_index(addr);
                Self::load_slice(&self.vram, VRAM_SIZE, idx, width)
            }
            0x07 => Self::load_slice(&self.oam, OAM_SIZE, addr as usize, width),
            0x08..=0x0D => {
                let offset = (addr & 0x01FF_FFFF) as usize;
                if offset + Self::width_bytes(width) as usize <= self.rom.len() {
                    Self::load_slice(&self.rom, self.rom.len(), offset, width)
                } else {
                    // Out-of-bounds cartridge read: the bus floats with the
                    // last prefetched halfword, which reads back as addr/2.
                    let half = |a: u32| u32::from((a >> 1) as u16);
                    match width {
                        Width::Word => half(addr) | half(addr + 2) << 16,
                        Width::Half => half(addr),
                        Width::Byte => {
                            let h = half(addr & !1);
                            if addr & 1 == 0 {
                                h & 0xFF
                            } else {
                                h >> 8
                            }
                        }
                    }
                }
            }
            // SRAM/Flash/EEPROM: stubbed until the cartridge module exists.
            0x0E | 0x0F => 0,
            _ => self.open_bus,
        }
    }

    /// Decode and perform a write of the given width, without timing.
    fn decode_write(&mut self, addr: u32, width: Width, value: u32) {
        match addr >> 24 {
            0x02 => Self::store_slice(&mut self.ewram, EWRAM_SIZE, addr as usize, width, value),
            0x03 => Self::store_slice(&mut self.iwram, IWRAM_SIZE, addr as usize, width, value),
            0x04 => self.io_write(addr & 0x3FF, width, value),
            0x05 => {
                // Palette RAM has a 16-bit bus: an 8-bit write duplicates the
                // byte across the whole halfword.
                if width == Width::Byte {
                    let dup = u32::from(value as u8) * 0x0101;
                    Self::store_slice(
                        &mut self.palette,
                        PALETTE_SIZE,
                        addr as usize & !1,
                        Width::Half,
                        dup,
                    );
                } else {
                    Self::store_slice(&mut self.palette, PALETTE_SIZE, addr as usize, width, value);
                }
            }
            0x06 => {
                let idx = Self::vram_index(addr);
                if width == Width::Byte {
                    // VRAM 8-bit write duplicates across the halfword in the BG
                    // area and is ignored in the OBJ area. The BG/OBJ boundary
                    // is mode-dependent: OBJ VRAM starts at 0x10000 in tiled
                    // modes and 0x14000 in the bitmap modes (3–5).
                    let obj_start = if self.ppu.bg_mode() >= 3 {
                        0x14000
                    } else {
                        0x10000
                    };
                    if idx < obj_start {
                        let dup = u32::from(value as u8) * 0x0101;
                        Self::store_slice(&mut self.vram, VRAM_SIZE, idx & !1, Width::Half, dup);
                    }
                } else {
                    Self::store_slice(&mut self.vram, VRAM_SIZE, idx, width, value);
                }
            }
            // OAM has a 16-bit bus and ignores 8-bit writes entirely.
            0x07 if width != Width::Byte => {
                Self::store_slice(&mut self.oam, OAM_SIZE, addr as usize, width, value)
            }
            0x07 => {}
            // BIOS and ROM are not writable; SRAM saves are stubbed for now.
            _ => {}
        }
    }

    /// Dispatch an I/O read (offset within 0x000..0x400) to the PPU, DMA,
    /// timer, or general I/O register file.
    fn io_read(&mut self, off: u32, width: Width) -> u32 {
        if off < PPU_REG_END {
            match width {
                Width::Byte => u32::from(self.ppu.read8(off)),
                Width::Half => u32::from(self.ppu.read16(off)),
                Width::Word => self.ppu.read32(off),
            }
        } else if (SOUND_REG_START..SOUND_REG_END).contains(&off) {
            match width {
                Width::Byte => u32::from(self.apu.read8(off)),
                Width::Half => u32::from(self.apu.read16(off)),
                Width::Word => self.apu.read32(off),
            }
        } else if (DMA_REG_START..DMA_REG_END).contains(&off) {
            match width {
                Width::Byte => u32::from(self.dma.read8(off)),
                Width::Half => u32::from(self.dma.read16(off)),
                Width::Word => self.dma.read32(off),
            }
        } else if (TIMER_REG_START..TIMER_REG_END).contains(&off) {
            match width {
                Width::Byte => u32::from(self.timers.read8(off)),
                Width::Half => u32::from(self.timers.read16(off)),
                Width::Word => self.timers.read32(off),
            }
        } else {
            match width {
                Width::Byte => u32::from(self.io.read8(off)),
                Width::Half => u32::from(self.io.read16(off)),
                Width::Word => self.io.read32(off),
            }
        }
    }

    fn io_write(&mut self, off: u32, width: Width, value: u32) {
        if off < PPU_REG_END {
            match width {
                Width::Byte => self.ppu.write8(off, value as u8),
                Width::Half => self.ppu.write16(off, value as u16),
                Width::Word => self.ppu.write32(off, value),
            }
        } else if (SOUND_REG_START..SOUND_REG_END).contains(&off) {
            match width {
                Width::Byte => self.apu.write8(off, value as u8),
                Width::Half => self.apu.write16(off, value as u16),
                Width::Word => self.apu.write32(off, value),
            }
        } else if (DMA_REG_START..DMA_REG_END).contains(&off) {
            match width {
                Width::Byte => self.dma.write8(off, value as u8),
                Width::Half => self.dma.write16(off, value as u16),
                Width::Word => self.dma.write32(off, value),
            }
        } else if (TIMER_REG_START..TIMER_REG_END).contains(&off) {
            match width {
                Width::Byte => self.timers.write8(off, value as u8),
                Width::Half => self.timers.write16(off, value as u16),
                Width::Word => self.timers.write32(off, value),
            }
        } else {
            match width {
                Width::Byte => self.io.write8(off, value as u8),
                Width::Half => self.io.write16(off, value as u16),
                Width::Word => self.io.write32(off, value),
            }
        }
    }

    fn timed_read(&mut self, addr: u32, width: Width) -> u32 {
        let seq = self.seq.classify(addr, width);
        self.access_cycles += access_cycles(Region::of(addr), width, seq, self.io.waitcnt());
        let value = self.decode_read(addr, width);
        self.open_bus = value;
        value
    }

    fn timed_write(&mut self, addr: u32, width: Width, value: u32) {
        let seq = self.seq.classify(addr, width);
        self.access_cycles += access_cycles(Region::of(addr), width, seq, self.io.waitcnt());
        self.open_bus = value;
        self.decode_write(addr, width, value);
    }
}

impl Bus for Memory {
    fn read8(&mut self, addr: u32) -> u8 {
        self.timed_read(addr, Width::Byte) as u8
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.timed_read(addr & !1, Width::Half) as u16
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.timed_read(addr & !3, Width::Word)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.timed_write(addr, Width::Byte, u32::from(value));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.timed_write(addr & !1, Width::Half, u32::from(value));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.timed_write(addr & !3, Width::Word, value);
    }

    fn has_bios(&self) -> bool {
        self.has_bios
    }

    fn irq_pending(&self) -> bool {
        self.io.irq_pending()
    }

    fn take_halt_request(&mut self) -> bool {
        self.io.take_halt_request()
    }

    fn take_access_cycles(&mut self) -> u32 {
        std::mem::take(&mut self.access_cycles)
    }
}
