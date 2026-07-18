//! Minimal GBA memory map — just enough for the CPU to run headlessly.
//!
//! Phase 1 deliberately stubs everything that is not RAM or ROM:
//!
//! * BIOS (16 KiB at `0x0000_0000`) — zero-filled unless a real BIOS image is
//!   loaded. ROMs that call SWI routines need a real BIOS.
//! * EWRAM / IWRAM — fully functional, with hardware-accurate mirroring.
//! * Palette / VRAM / OAM — mapped as plain RAM so test ROMs that render
//!   results can run (and so a future test can peek at VRAM), but no PPU
//!   semantics (no 8-bit-write quirks, no access timing).
//! * I/O registers — reads return benign values; `KEYINPUT` reads as "no keys
//!   pressed", and `DISPSTAT`/`VCOUNT` fake scanline progress so busy-wait
//!   vblank loops terminate headlessly. Writes are ignored.
//! * Wait states / open bus / prefetch buffer — not modeled yet; that work
//!   belongs to Phase 2 (memory/bus) where it can be done properly. The one
//!   exception is out-of-bounds cartridge reads, which return the documented
//!   `addr / 2` open-bus pattern because cheap and some ROMs depend on it.

use crate::CoreError;
use std::path::Path;

pub const BIOS_SIZE: usize = 16 * 1024;
pub const EWRAM_SIZE: usize = 256 * 1024;
pub const IWRAM_SIZE: usize = 32 * 1024;
pub const PALETTE_SIZE: usize = 1024;
pub const VRAM_SIZE: usize = 96 * 1024;
pub const OAM_SIZE: usize = 1024;
pub const ROM_MAX_SIZE: usize = 32 * 1024 * 1024;

/// CPU-visible address space. The CPU is generic over this trait so tests can
/// substitute flat RAM, and later phases can layer in the PPU/APU/DMA without
/// touching the CPU.
///
/// The 16/32-bit accessors have byte-composed default implementations;
/// implementors only *must* provide byte access. Addresses are force-aligned
/// here; misaligned-access rotation quirks are CPU behavior and live in the
/// CPU core.
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
}

/// The GBA memory map (Phase 1 subset).
pub struct Memory {
    bios: Vec<u8>,
    ewram: Vec<u8>,
    iwram: Vec<u8>,
    palette: Vec<u8>,
    vram: Vec<u8>,
    oam: Vec<u8>,
    rom: Vec<u8>,
    /// Counts reads of DISPSTAT/VCOUNT to fake scanline progress headlessly.
    io_poll: u32,
}

impl Memory {
    /// Create a memory map with the given cartridge ROM and a zero-filled BIOS.
    pub fn new(rom: Vec<u8>) -> Result<Self, CoreError> {
        if rom.len() > ROM_MAX_SIZE {
            return Err(CoreError::RomTooLarge { size: rom.len(), max: ROM_MAX_SIZE });
        }
        Ok(Self {
            bios: vec![0; BIOS_SIZE],
            ewram: vec![0; EWRAM_SIZE],
            iwram: vec![0; IWRAM_SIZE],
            palette: vec![0; PALETTE_SIZE],
            vram: vec![0; VRAM_SIZE],
            oam: vec![0; OAM_SIZE],
            rom,
            io_poll: 0,
        })
    }

    /// Load a real BIOS image (must be exactly 16 KiB).
    pub fn load_bios(&mut self, bios: Vec<u8>) -> Result<(), CoreError> {
        if bios.len() != BIOS_SIZE {
            return Err(CoreError::BadBiosSize { size: bios.len(), expected: BIOS_SIZE });
        }
        self.bios = bios;
        Ok(())
    }

    /// Convenience constructor: load a ROM file, and optionally a BIOS file.
    pub fn from_files(rom: &Path, bios: Option<&Path>) -> Result<Self, CoreError> {
        let mut mem = Self::new(std::fs::read(rom)?)?;
        if let Some(b) = bios {
            mem.load_bios(std::fs::read(b)?)?;
        }
        Ok(mem)
    }

    /// Read-only view of VRAM, for future headless render checks.
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// VRAM is 96 KiB but mirrored in a 128 KiB window: the upper 32 KiB
    /// mirrors the 64K–96K region.
    fn vram_index(addr: u32) -> usize {
        let a = (addr & 0x1_FFFF) as usize;
        if a >= 0x18000 {
            a - 0x8000
        } else {
            a
        }
    }

    /// Fake VCOUNT that advances as software polls it, so headless ROMs that
    /// spin on vblank make progress. Replaced by real PPU timing in Phase 3.
    fn vcount(&self) -> u32 {
        (self.io_poll / 8) % 228
    }

    fn io_read8(&mut self, addr: u32) -> u8 {
        match addr & 0xFFFF {
            // DISPSTAT: bit 0 = vblank flag.
            0x0004 => {
                self.io_poll = self.io_poll.wrapping_add(1);
                u8::from(self.vcount() >= 160)
            }
            0x0005 => 0,
            // VCOUNT
            0x0006 => {
                self.io_poll = self.io_poll.wrapping_add(1);
                self.vcount() as u8
            }
            0x0007 => 0,
            // KEYINPUT: bits are active-low; 0x03FF = no keys pressed.
            0x0130 => 0xFF,
            0x0131 => 0x03,
            _ => 0,
        }
    }
}

impl Bus for Memory {
    fn read8(&mut self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 => self.bios.get(addr as usize).copied().unwrap_or(0),
            0x02 => self.ewram[(addr as usize) & (EWRAM_SIZE - 1)],
            0x03 => self.iwram[(addr as usize) & (IWRAM_SIZE - 1)],
            0x04 => self.io_read8(addr),
            0x05 => self.palette[(addr as usize) & (PALETTE_SIZE - 1)],
            0x06 => self.vram[Self::vram_index(addr)],
            0x07 => self.oam[(addr as usize) & (OAM_SIZE - 1)],
            0x08..=0x0D => {
                let offset = (addr & 0x01FF_FFFF) as usize;
                match self.rom.get(offset) {
                    Some(&b) => b,
                    // Out-of-bounds cartridge reads: the bus floats with the
                    // last prefetched value, which is `addr / 2` per halfword.
                    None => {
                        let half = ((addr & !1) >> 1) as u16;
                        if addr & 1 == 0 {
                            half as u8
                        } else {
                            (half >> 8) as u8
                        }
                    }
                }
            }
            // SRAM/Flash: stubbed until the cartridge module exists.
            0x0E | 0x0F => 0,
            _ => 0,
        }
    }

    fn write8(&mut self, addr: u32, value: u8) {
        match addr >> 24 {
            0x02 => self.ewram[(addr as usize) & (EWRAM_SIZE - 1)] = value,
            0x03 => self.iwram[(addr as usize) & (IWRAM_SIZE - 1)] = value,
            0x05 => self.palette[(addr as usize) & (PALETTE_SIZE - 1)] = value,
            0x06 => self.vram[Self::vram_index(addr)] = value,
            0x07 => self.oam[(addr as usize) & (OAM_SIZE - 1)] = value,
            // BIOS, ROM and (for now) I/O are not writable.
            _ => {}
        }
    }
}
