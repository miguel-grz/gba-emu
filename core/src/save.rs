//! Cartridge save memory: SRAM and Flash (the types Pokémon and most GBA RPGs
//! use). The save type is auto-detected from an ID string in the ROM.
//!
//! Flash uses a small command protocol (unlock writes to 0x5555/0x2AAA, then a
//! command byte): the game reads the chip's manufacturer/device ID to identify
//! it, erases sectors, programs bytes, and — for the 128 KiB part — switches
//! between two 64 KiB banks. EEPROM (a DMA-driven serial protocol) is not
//! implemented yet; those games fall back to no save.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveType {
    None,
    Sram,
    Flash64,
    Flash128,
}

/// Detect the save type from the ID marker most GBA ROMs embed.
pub fn detect(rom: &[u8]) -> SaveType {
    let has = |needle: &[u8]| rom.windows(needle.len()).any(|w| w == needle);
    if has(b"FLASH1M_V") || has(b"FLASH1M") {
        SaveType::Flash128
    } else if has(b"FLASH512_V") || has(b"FLASH_V") || has(b"FLASH") {
        SaveType::Flash64
    } else if has(b"SRAM_V") || has(b"SRAM_F_V") || has(b"SRAM") {
        SaveType::Sram
    } else {
        // Default to SRAM: harmless for games without a save, and correct for
        // the many that use plain SRAM without an obvious marker.
        SaveType::Sram
    }
}

#[derive(Serialize, Deserialize)]
pub struct Save {
    kind: SaveType,
    data: Vec<u8>,
    // Flash command state.
    bank: usize,
    id_mode: bool,
    erase_armed: bool,
    write_armed: bool,
    bank_armed: bool,
    cmd_step: u8,
    man_id: u8,
    dev_id: u8,
}

impl Save {
    pub fn new(kind: SaveType) -> Self {
        let (size, man_id, dev_id) = match kind {
            SaveType::None => (0, 0, 0),
            SaveType::Sram => (0x8000, 0, 0),
            // Panasonic 64 KiB flash.
            SaveType::Flash64 => (0x10000, 0x32, 0x1B),
            // Sanyo 128 KiB flash (two 64 KiB banks) — accepted by Pokémon.
            SaveType::Flash128 => (0x20000, 0x62, 0x13),
        };
        Save {
            kind,
            data: vec![0xFF; size],
            bank: 0,
            id_mode: false,
            erase_armed: false,
            write_armed: false,
            bank_armed: false,
            cmd_step: 0,
            man_id,
            dev_id,
        }
    }

    pub fn kind(&self) -> SaveType {
        self.kind
    }

    /// The backing bytes, for persisting the battery save.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn load_data(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.data.len());
        self.data[..n].copy_from_slice(&bytes[..n]);
    }

    pub fn read(&self, addr: u32) -> u8 {
        match self.kind {
            SaveType::None => 0xFF,
            SaveType::Sram => {
                let mask = self.data.len().max(1) - 1;
                self.data[(addr as usize) & mask]
            }
            SaveType::Flash64 | SaveType::Flash128 => {
                let off = (addr & 0xFFFF) as usize;
                if self.id_mode {
                    match off {
                        0 => self.man_id,
                        1 => self.dev_id,
                        _ => 0xFF,
                    }
                } else {
                    self.data[self.bank * 0x10000 + off]
                }
            }
        }
    }

    pub fn write(&mut self, addr: u32, value: u8) {
        match self.kind {
            SaveType::None => {}
            SaveType::Sram => {
                let mask = self.data.len().max(1) - 1;
                self.data[(addr as usize) & mask] = value;
            }
            SaveType::Flash64 | SaveType::Flash128 => self.flash_write(addr, value),
        }
    }

    fn flash_write(&mut self, addr: u32, value: u8) {
        let off = addr & 0xFFFF;
        // A pending single-byte program: flash can only clear bits (AND).
        if self.write_armed {
            self.data[self.bank * 0x10000 + off as usize] &= value;
            self.write_armed = false;
            self.cmd_step = 0;
            return;
        }
        // A pending bank switch (128 KiB only): the write to 0x0000 selects it.
        if self.bank_armed {
            if off == 0 {
                self.bank = (value & 1) as usize;
            }
            self.bank_armed = false;
            self.cmd_step = 0;
            return;
        }
        match self.cmd_step {
            0 if off == 0x5555 && value == 0xAA => self.cmd_step = 1,
            1 if off == 0x2AAA && value == 0x55 => self.cmd_step = 2,
            2 => {
                if off == 0x5555 {
                    self.flash_command(value);
                } else if self.erase_armed && value == 0x30 {
                    // Sector erase (4 KiB) at the addressed sector.
                    let base = self.bank * 0x10000 + (off as usize & 0xF000);
                    for b in &mut self.data[base..base + 0x1000] {
                        *b = 0xFF;
                    }
                    self.erase_armed = false;
                }
                self.cmd_step = 0;
            }
            _ => self.cmd_step = 0,
        }
    }

    fn flash_command(&mut self, cmd: u8) {
        match cmd {
            0x90 => self.id_mode = true,
            0xF0 => self.id_mode = false,
            0x80 => self.erase_armed = true,
            0xA0 => self.write_armed = true,
            0xB0 if self.kind == SaveType::Flash128 => self.bank_armed = true,
            0x10 if self.erase_armed => {
                self.data.fill(0xFF); // chip erase
                self.erase_armed = false;
            }
            _ => {}
        }
    }
}
