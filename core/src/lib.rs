//! Game Boy Advance emulator core.
//!
//! Phase 1 scope: the ARM7TDMI CPU (ARM + Thumb instruction sets, banked
//! registers, CPSR/SPSR, pipeline behavior) plus a minimal memory map that is
//! just enough to execute code headlessly. The PPU, APU, DMA, timers and
//! cartridge save handling arrive in later phases; the [`memory::Bus`] trait
//! is the seam they will plug into without requiring CPU changes.

pub mod bios;
pub mod cpu;
pub mod dma;
pub mod io;
pub mod memory;
pub mod ppu;
pub mod system;
pub mod timers;
pub mod timing;

pub use cpu::{Cpu, Mode};
pub use memory::{Bus, Memory};
pub use ppu::Ppu;
pub use system::Gba;

use std::fmt;

/// Errors surfaced by the emulator core.
#[derive(Debug)]
#[non_exhaustive]
pub enum CoreError {
    /// ROM image exceeds the 32 MiB cartridge address space.
    RomTooLarge { size: usize, max: usize },
    /// BIOS image is not exactly 16 KiB.
    BadBiosSize { size: usize, expected: usize },
    /// Underlying I/O failure while loading a file.
    Io(std::io::Error),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::RomTooLarge { size, max } => {
                write!(
                    f,
                    "ROM is {size} bytes, exceeding the {max}-byte cartridge limit"
                )
            }
            CoreError::BadBiosSize { size, expected } => {
                write!(f, "BIOS is {size} bytes, expected exactly {expected}")
            }
            CoreError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for CoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CoreError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io(e)
    }
}
