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

use crate::io::Io;
use crate::timing::{access_cycles, Region, SeqTracker, Width};
use crate::CoreError;
use std::path::Path;

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

    /// Read-only view of VRAM, for future headless render checks.
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
            0x04 => match width {
                Width::Byte => u32::from(self.io.read8(addr & 0x3FF)),
                Width::Half => u32::from(self.io.read16(addr & 0x3FF)),
                Width::Word => self.io.read32(addr & 0x3FF),
            },
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
            0x04 => match width {
                Width::Byte => self.io.write8(addr & 0x3FF, value as u8),
                Width::Half => self.io.write16(addr & 0x3FF, value as u16),
                Width::Word => self.io.write32(addr & 0x3FF, value),
            },
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
                    // is mode-dependent (0x14000 in tiled modes); we use that
                    // boundary now and refine it with the PPU in Phase 3.
                    if idx < 0x14000 {
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
