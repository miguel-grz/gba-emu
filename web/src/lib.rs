//! WebAssembly bindings: a thin JS-facing wrapper around the emulator core.
//!
//! The frontend drives the loop — one `run_frame` per animation frame, reads
//! the RGBA framebuffer into a canvas, pushes the audio samples into WebAudio,
//! and feeds keypad state back in.

use gba_core::apu::SAMPLE_RATE;
use gba_core::ppu::{bgr555_to_rgb888, SCREEN_H, SCREEN_W};
use gba_core::Gba;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Emulator {
    gba: Gba,
    /// Reused RGBA scratch buffer (240 × 160 × 4).
    frame: Vec<u8>,
}

#[wasm_bindgen]
impl Emulator {
    /// Boot a cartridge from its ROM bytes (HLE BIOS, no BIOS image needed).
    #[wasm_bindgen(constructor)]
    pub fn new(rom: Vec<u8>) -> Result<Emulator, JsValue> {
        let gba = Gba::new(rom).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Emulator {
            gba,
            frame: vec![0; SCREEN_W * SCREEN_H * 4],
        })
    }

    pub fn width() -> u32 {
        SCREEN_W as u32
    }

    pub fn height() -> u32 {
        SCREEN_H as u32
    }

    pub fn sample_rate() -> u32 {
        SAMPLE_RATE
    }

    /// Run the emulator until the next full frame is drawn.
    pub fn run_frame(&mut self) {
        self.gba.run_frame(1_000_000);
    }

    /// The current frame as RGBA8888 bytes (row-major, 240 × 160).
    pub fn frame(&mut self) -> Vec<u8> {
        for (i, &px) in self.gba.framebuffer().iter().enumerate() {
            let (r, g, b) = bgr555_to_rgb888(px);
            self.frame[i * 4] = r;
            self.frame[i * 4 + 1] = g;
            self.frame[i * 4 + 2] = b;
            self.frame[i * 4 + 3] = 255;
        }
        self.frame.clone()
    }

    /// Take the accumulated stereo audio samples (interleaved L/R, i16).
    pub fn drain_samples(&mut self) -> Vec<i16> {
        self.gba.drain_samples()
    }

    /// Set the pressed buttons, active-high, in KEYINPUT order:
    /// bit 0 A, 1 B, 2 Select, 3 Start, 4 Right, 5 Left, 6 Up, 7 Down, 8 R, 9 L.
    pub fn set_keys(&mut self, pressed: u16) {
        self.gba.set_keys(pressed);
    }
}
