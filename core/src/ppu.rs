//! Picture Processing Unit — Phase 3: tiled (text) background rendering.
//!
//! This implements the **text background layers** of BG modes 0–2, a
//! scanline-based renderer, and the display timing that raises the
//! VBlank/HBlank/VCount interrupts. What that covers:
//!
//! * Mode 0 — four text backgrounds (BG0–BG3).
//! * Mode 1 — two text backgrounds (BG0, BG1); BG2 is affine (Phase 5).
//! * Mode 2 — both backgrounds are affine (Phase 5), so nothing tiled renders.
//! * 4bpp (16×16-color) and 8bpp (256-color) tiles, per-tile H/V flip,
//!   per-background scroll, all four text map sizes, and priority compositing
//!   over the backdrop.
//!
//! Deferred to later phases (and cleanly separable): sprites/OBJ (Phase 4),
//! affine backgrounds, the bitmap modes 3–5, windows, mosaic, and alpha
//! blending (Phase 5). The register storage for those already exists here so
//! reads/writes behave; only their *rendering* is missing.
//!
//! ### Timing
//!
//! A scanline is 1232 cycles: 960 of HDraw then 272 of HBlank; 228 lines per
//! frame (160 visible + 68 of VBlank). [`Ppu::tick`] advances a per-cycle dot
//! counter and emits interrupts at the HBlank, VBlank and VCount-match events.
//! Rendering happens once per visible line at the start of its HBlank, which
//! is where real hardware has finished drawing it.

use crate::io::irq;

pub const SCREEN_W: usize = 240;
pub const SCREEN_H: usize = 160;
const DOTS_PER_LINE: u32 = 1232;
const HDRAW_DOTS: u32 = 960;
const TOTAL_LINES: u16 = 228;

/// Number of 16-bit registers in the PPU's I/O block (0x000..0x060).
const NUM_REGS: usize = 0x30;

pub struct Ppu {
    /// Raw 16-bit register file, indexed by `(offset & 0x3F) >> 1`. Typed
    /// accessors below interpret the fields; unimplemented registers (affine,
    /// window, mosaic, blend) are stored here so reads return what was written.
    regs: [u16; NUM_REGS],
    /// Current scanline (0..228). Drives VCOUNT; not stored in `regs`.
    vcount: u16,
    /// Cycle within the current scanline (0..1232).
    dot: u32,
    /// 240×160 framebuffer in 15-bit BGR555 (bit 15 unused).
    framebuffer: Vec<u16>,
    /// Set when a full visible frame has been drawn (entering VBlank).
    frame_ready: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            regs: [0; NUM_REGS],
            vcount: 0,
            dot: 0,
            framebuffer: vec![0; SCREEN_W * SCREEN_H],
            frame_ready: false,
        }
    }

    /// The 15-bit BGR555 framebuffer (240×160, row-major).
    pub fn framebuffer(&self) -> &[u16] {
        &self.framebuffer
    }

    /// Take (and clear) the "a frame finished drawing" flag.
    pub fn take_frame_ready(&mut self) -> bool {
        std::mem::replace(&mut self.frame_ready, false)
    }

    pub fn vcount(&self) -> u16 {
        self.vcount
    }

    // ---- register access (0x000..0x060) ----

    fn dispcnt(&self) -> u16 {
        self.regs[0]
    }
    fn dispstat(&self) -> u16 {
        self.regs[2]
    }
    fn bgcnt(&self, bg: usize) -> u16 {
        self.regs[4 + bg]
    }
    fn bghofs(&self, bg: usize) -> u16 {
        self.regs[8 + bg * 2] & 0x1FF
    }
    fn bgvofs(&self, bg: usize) -> u16 {
        self.regs[9 + bg * 2] & 0x1FF
    }

    /// Live DISPSTAT: the writable enable/setting bits OR the computed status
    /// bits (VBlank/HBlank/VCount-match), which are read-only.
    fn dispstat_read(&self) -> u16 {
        let mut status = 0;
        if (160..227).contains(&self.vcount) {
            status |= 1 << 0; // VBlank
        }
        if self.dot >= HDRAW_DOTS {
            status |= 1 << 1; // HBlank
        }
        if self.vcount == self.dispstat() >> 8 {
            status |= 1 << 2; // VCount match
        }
        (self.dispstat() & 0xFF38) | status
    }

    pub fn read16(&self, offset: u32) -> u16 {
        match offset & 0x3E {
            0x04 => self.dispstat_read(),
            0x06 => self.vcount,
            other => self.regs[(other >> 1) as usize],
        }
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        match offset & 0x3E {
            // DISPSTAT: only the IRQ-enable bits (3–5) and VCount setting
            // (8–15) are writable; the status bits are hardware-driven.
            0x04 => {
                let mask = 0xFF38;
                self.regs[2] = (self.regs[2] & !mask) | (value & mask);
            }
            0x06 => {} // VCOUNT is read-only
            other => self.regs[(other >> 1) as usize] = value,
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

    // ---- timing ----

    /// Advance the PPU by `cycles`, rendering visible scanlines and returning
    /// the set of interrupt bits (VBlank/HBlank/VCount) to raise in `IF`.
    /// `vram`/`palette`/`oam` are borrowed from [`crate::memory::Memory`] for
    /// the duration of any rendering triggered here.
    pub fn tick(&mut self, cycles: u64, vram: &[u8], palette: &[u8], oam: &[u8]) -> u16 {
        let mut irqs = 0;
        for _ in 0..cycles {
            irqs |= self.tick_one(vram, palette, oam);
        }
        irqs
    }

    fn tick_one(&mut self, vram: &[u8], palette: &[u8], _oam: &[u8]) -> u16 {
        let mut irqs = 0;
        self.dot += 1;

        if self.dot == HDRAW_DOTS {
            // Entering HBlank: the line is now fully "drawn".
            if self.vcount < SCREEN_H as u16 {
                self.render_scanline(self.vcount, vram, palette);
            }
            if self.dispstat() & 1 << 4 != 0 {
                irqs |= irq::HBLANK;
            }
        }

        if self.dot >= DOTS_PER_LINE {
            self.dot = 0;
            self.vcount += 1;
            if self.vcount >= TOTAL_LINES {
                self.vcount = 0;
            }
            if self.vcount == SCREEN_H as u16 {
                self.frame_ready = true;
                if self.dispstat() & 1 << 3 != 0 {
                    irqs |= irq::VBLANK;
                }
            }
            if self.vcount == self.dispstat() >> 8 && self.dispstat() & 1 << 5 != 0 {
                irqs |= irq::VCOUNT;
            }
        }
        irqs
    }

    // ---- rendering ----

    /// Whether background `bg` is a text (tiled) layer in the current mode.
    fn is_text_bg(mode: u16, bg: usize) -> bool {
        match mode {
            0 => bg < 4,
            1 => bg < 2, // BG0/BG1 text; BG2 affine (Phase 5)
            _ => false,  // mode 2 affine; modes 3–5 bitmap (Phase 5)
        }
    }

    fn render_scanline(&mut self, ly: u16, vram: &[u8], palette: &[u8]) {
        let base = ly as usize * SCREEN_W;

        // Forced blank (DISPCNT bit 7) outputs white.
        if self.dispcnt() & 1 << 7 != 0 {
            self.framebuffer[base..base + SCREEN_W].fill(0x7FFF);
            return;
        }

        let backdrop = read_u16(palette, 0);
        let mut line = [backdrop; SCREEN_W];
        let mut written = [false; SCREEN_W];

        let mode = self.dispcnt() & 0x7;
        // Composite front-to-back: enabled text BGs ordered by (priority, index).
        let mut order: Vec<usize> = (0..4)
            .filter(|&bg| self.dispcnt() & (1 << (8 + bg)) != 0 && Self::is_text_bg(mode, bg))
            .collect();
        order.sort_by_key(|&bg| (self.bgcnt(bg) & 0x3, bg));

        for bg in order {
            self.render_text_bg_line(bg, ly, vram, palette, &mut line, &mut written);
        }

        self.framebuffer[base..base + SCREEN_W].copy_from_slice(&line);
    }

    /// Render one text background into the scanline buffer, filling only pixels
    /// not already claimed by a higher-priority layer.
    fn render_text_bg_line(
        &self,
        bg: usize,
        ly: u16,
        vram: &[u8],
        palette: &[u8],
        line: &mut [u16; SCREEN_W],
        written: &mut [bool; SCREEN_W],
    ) {
        let cnt = self.bgcnt(bg);
        let char_base = (((cnt >> 2) & 0x3) as usize) * 0x4000;
        let screen_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let color8 = cnt & 1 << 7 != 0;
        let size = (cnt >> 14) & 0x3;
        let (w_tiles, h_tiles) = match size {
            0 => (32, 32),
            1 => (64, 32),
            2 => (32, 64),
            _ => (64, 64),
        };
        let w_px = w_tiles * 8;
        let h_px = h_tiles * 8;

        let bgy = (ly as usize + self.bgvofs(bg) as usize) % h_px;
        let ty = bgy / 8;
        let py = bgy % 8;

        for x in 0..SCREEN_W {
            if written[x] {
                continue;
            }
            let bgx = (x + self.bghofs(bg) as usize) % w_px;
            let tx = bgx / 8;

            // Select the 32×32-tile screenblock, then the entry within it.
            let sb = (tx / 32) + (ty / 32) * (w_tiles / 32);
            let entry_idx = (ty % 32) * 32 + (tx % 32);
            let map_addr = screen_base + sb * 0x800 + entry_idx * 2;
            let entry = read_u16(vram, map_addr);

            let tile = (entry & 0x3FF) as usize;
            let hflip = entry & 1 << 10 != 0;
            let vflip = entry & 1 << 11 != 0;
            let pal_bank = ((entry >> 12) & 0xF) as usize;

            let fx = if hflip { 7 - (bgx % 8) } else { bgx % 8 };
            let fy = if vflip { 7 - py } else { py };

            let color_index = if color8 {
                vram[char_base + tile * 64 + fy * 8 + fx] as usize
            } else {
                let byte = vram[char_base + tile * 32 + fy * 4 + fx / 2];
                let nibble = if fx & 1 == 0 { byte & 0xF } else { byte >> 4 };
                nibble as usize
            };

            // Index 0 of the (sub-)palette is transparent.
            if color_index == 0 {
                continue;
            }
            let pal_entry = if color8 {
                color_index
            } else {
                pal_bank * 16 + color_index
            };
            line[x] = read_u16(palette, pal_entry * 2);
            written[x] = true;
        }
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

fn read_u16(data: &[u8], addr: usize) -> u16 {
    u16::from(data[addr]) | u16::from(data[addr + 1]) << 8
}

/// Convert a 15-bit BGR555 color to 8-bit-per-channel RGB (for the frontend
/// and headless PPM/PNG dumps). Channels are scaled from 5 to 8 bits.
pub fn bgr555_to_rgb888(color: u16) -> (u8, u8, u8) {
    let r = (color & 0x1F) as u8;
    let g = ((color >> 5) & 0x1F) as u8;
    let b = ((color >> 10) & 0x1F) as u8;
    let expand = |c: u8| (c << 3) | (c >> 2);
    (expand(r), expand(g), expand(b))
}
