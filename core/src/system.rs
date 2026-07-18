//! Top-level system: ties the CPU and memory (with its PPU) together and
//! drives the main loop. This is the seam the frontend will call into.

use crate::cpu::Cpu;
use crate::memory::Memory;
use crate::CoreError;
use std::path::Path;

/// A complete (Phase-3) GBA: CPU + memory bus + PPU.
pub struct Gba {
    pub cpu: Cpu,
    pub mem: Memory,
}

impl Gba {
    /// Boot a cartridge in HLE mode (post-BIOS state, no BIOS image needed).
    pub fn new(rom: Vec<u8>) -> Result<Self, CoreError> {
        let mut mem = Memory::new(rom)?;
        let mut cpu = Cpu::new();
        cpu.skip_bios(&mut mem);
        Ok(Gba { cpu, mem })
    }

    /// Boot a cartridge from files, optionally with a real BIOS (LLE).
    pub fn from_files(rom: &Path, bios: Option<&Path>) -> Result<Self, CoreError> {
        let mut mem = Memory::from_files(rom, bios)?;
        let mut cpu = Cpu::new();
        cpu.skip_bios(&mut mem);
        Ok(Gba { cpu, mem })
    }

    /// Execute one CPU instruction and advance the PPU by the cycles it took.
    pub fn step(&mut self) -> u64 {
        let cycles = self.cpu.step(&mut self.mem);
        self.mem.tick(cycles);
        cycles
    }

    /// Run until the PPU signals a completed frame, or `max_steps` is reached
    /// (a guard against ROMs that never reach VBlank). Returns the steps run.
    pub fn run_frame(&mut self, max_steps: u64) -> u64 {
        let mut steps = 0;
        while steps < max_steps {
            self.step();
            steps += 1;
            if self.mem.take_frame_ready() {
                break;
            }
        }
        steps
    }

    /// The current 15-bit BGR555 framebuffer (240×160).
    pub fn framebuffer(&self) -> &[u16] {
        self.mem.framebuffer()
    }

    /// Take the accumulated stereo audio samples (interleaved L/R, i16).
    pub fn drain_samples(&mut self) -> Vec<i16> {
        self.mem.drain_samples()
    }

    /// Set the pressed buttons (active-high, KEYINPUT bit order: A, B, Select,
    /// Start, Right, Left, Up, Down, R, L).
    pub fn set_keys(&mut self, pressed: u16) {
        self.mem.set_keys(pressed);
    }

    /// The cartridge battery save memory (persist this as the game's `.sav`).
    pub fn save_data(&self) -> &[u8] {
        self.mem.save_data()
    }

    /// Restore battery save memory from a persisted `.sav`.
    pub fn load_save_data(&mut self, bytes: &[u8]) {
        self.mem.load_save_data(bytes);
    }

    /// Serialize a save state (CPU + memory + peripherals, minus the ROM).
    pub fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        let mut out =
            bincode::serialize(&self.cpu).map_err(|e| CoreError::SaveState(e.to_string()))?;
        let mem = self.mem.save_state()?;
        // Length-prefix the CPU blob so load can split the two.
        let mut blob = (out.len() as u32).to_le_bytes().to_vec();
        blob.append(&mut out);
        blob.extend_from_slice(&mem);
        Ok(blob)
    }

    /// Restore a save state produced by [`Gba::save_state`], keeping the ROM.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), CoreError> {
        if data.len() < 4 {
            return Err(CoreError::SaveState("truncated".into()));
        }
        let cpu_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let end = 4 + cpu_len;
        if data.len() < end {
            return Err(CoreError::SaveState("truncated".into()));
        }
        self.cpu =
            bincode::deserialize(&data[4..end]).map_err(|e| CoreError::SaveState(e.to_string()))?;
        self.mem.load_state(&data[end..])
    }
}
