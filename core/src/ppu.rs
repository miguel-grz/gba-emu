//! Picture Processing Unit — Phase 3: tiled (text) background rendering.
//!
//! This implements the **text background layers** of BG modes 0–2, a
//! scanline-based renderer, and the display timing that raises the
//! VBlank/HBlank/VCount interrupts. What that covers:
//!
//! * Mode 0 — four text backgrounds (BG0–BG3).
//! * Mode 1 — two text backgrounds (BG0, BG1) plus affine BG2.
//! * Mode 2 — affine BG2 and BG3.
//! * 4bpp/8bpp tiles, per-tile H/V flip, per-background scroll, all four text
//!   map sizes, and priority compositing over the backdrop.
//! * Affine backgrounds (rotation/scaling, per-scanline reference updates,
//!   wrap vs transparent); see [`Ppu::render_affine_bg_line`].
//! * Sprites (OBJ), regular and affine incl. double-size; see
//!   [`Ppu::render_sprites_line`].
//! * BG and OBJ mosaic.
//! * The bitmap modes 3–5 (BG2); see [`Ppu::render_bitmap_line`].
//! * Windows (WIN0/WIN1/OBJ window) and the color special effects — alpha
//!   blending, brighten, darken, and OBJ semi-transparency; see
//!   [`Ppu::composite`].
//!
//! Compositing renders each layer into its own buffer, then per pixel selects
//! the front-most and second layers (honoring window masks) and applies the
//! blend/brightness effect. This is the whole GBA PPU feature set for the tiled
//! and bitmap modes.
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

/// Result of advancing the PPU: interrupts to raise, plus the display events
/// that drive DMA (independent of the interrupt-enable bits).
#[derive(Default)]
pub struct PpuOutput {
    pub irqs: u16,
    pub vblank_start: bool,
    pub hblank_start: bool,
}

/// How a background layer is rendered in a given tiled mode.
enum BgKind {
    Text,
    Affine,
}

/// One rendered background scanline: a color per pixel and whether it is
/// opaque (index-0 / out-of-map pixels are transparent).
struct Layer {
    color: [u16; SCREEN_W],
    opaque: [bool; SCREEN_W],
}

impl Layer {
    fn new() -> Self {
        Layer {
            color: [0; SCREEN_W],
            opaque: [false; SCREEN_W],
        }
    }
}

/// The rendered sprite scanline, with the extra per-pixel state compositing
/// needs: priority, the semi-transparent (blend) flag, and OBJ-window coverage.
struct ObjLayer {
    color: [u16; SCREEN_W],
    opaque: [bool; SCREEN_W],
    prio: [u8; SCREEN_W],
    semi: [bool; SCREEN_W],
    window: [bool; SCREEN_W],
}

impl ObjLayer {
    fn new() -> Self {
        ObjLayer {
            color: [0; SCREEN_W],
            opaque: [false; SCREEN_W],
            prio: [0; SCREEN_W],
            semi: [false; SCREEN_W],
            window: [false; SCREEN_W],
        }
    }
}
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
    /// Internal affine reference points for BG2 (`[0]`) and BG3 (`[1]`), in
    /// 20.8 fixed point. Reloaded from BGxX/BGxY each frame and on write, and
    /// advanced by (PB, PD) after every visible scanline.
    ref_x: [i32; 2],
    ref_y: [i32; 2],
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            regs: [0; NUM_REGS],
            vcount: 0,
            dot: 0,
            framebuffer: vec![0; SCREEN_W * SCREEN_H],
            frame_ready: false,
            ref_x: [0; 2],
            ref_y: [0; 2],
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

    /// Current BG mode (DISPCNT bits 0–2). Used by the memory bus to place the
    /// mode-dependent BG/OBJ boundary for VRAM 8-bit writes.
    pub fn bg_mode(&self) -> u16 {
        self.dispcnt() & 0x7
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

    // Affine parameters for BG2 (`k`=0) and BG3 (`k`=1); PA–PD are signed 8.8.
    fn bg_pa(&self, k: usize) -> i32 {
        self.regs[0x10 + k * 8] as i16 as i32
    }
    fn bg_pb(&self, k: usize) -> i32 {
        self.regs[0x11 + k * 8] as i16 as i32
    }
    fn bg_pc(&self, k: usize) -> i32 {
        self.regs[0x12 + k * 8] as i16 as i32
    }
    fn bg_pd(&self, k: usize) -> i32 {
        self.regs[0x13 + k * 8] as i16 as i32
    }

    /// Reload the internal affine reference point of BG (2+`k`) from its
    /// BGxX/BGxY registers (28-bit signed, 20.8 fixed point).
    fn reload_ref(&mut self, k: usize) {
        let base = 0x14 + k * 8;
        let x = u32::from(self.regs[base]) | u32::from(self.regs[base + 1]) << 16;
        let y = u32::from(self.regs[base + 2]) | u32::from(self.regs[base + 3]) << 16;
        self.ref_x[k] = sign_extend_28(x);
        self.ref_y[k] = sign_extend_28(y);
    }

    fn mosaic(&self) -> u16 {
        self.regs[0x26]
    }

    /// BG mosaic (horizontal, vertical) block sizes for `bg`, or (1, 1) when
    /// that background's mosaic bit is clear.
    fn bg_mosaic(&self, bg: usize) -> (usize, usize) {
        if self.bgcnt(bg) & 1 << 6 != 0 {
            let m = self.mosaic();
            ((m & 0xF) as usize + 1, ((m >> 4) & 0xF) as usize + 1)
        } else {
            (1, 1)
        }
    }

    /// OBJ mosaic (horizontal, vertical) block sizes.
    fn obj_mosaic(&self) -> (usize, usize) {
        let m = self.mosaic();
        (
            ((m >> 8) & 0xF) as usize + 1,
            ((m >> 12) & 0xF) as usize + 1,
        )
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

    /// Index into `regs` for a byte offset (the block is 0x60 bytes, which is
    /// not a power of two, so a bitmask would alias — index modulo instead).
    fn reg_index(offset: u32) -> usize {
        (offset >> 1) as usize % NUM_REGS
    }

    pub fn read16(&self, offset: u32) -> u16 {
        match offset {
            0x04 => self.dispstat_read(),
            0x06 => self.vcount,
            _ => self.regs[Self::reg_index(offset)],
        }
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        match offset {
            // DISPSTAT: only the IRQ-enable bits (3–5) and VCount setting
            // (8–15) are writable; the status bits are hardware-driven.
            0x04 => {
                let mask = 0xFF38;
                self.regs[2] = (self.regs[2] & !mask) | (value & mask);
            }
            0x06 => {} // VCOUNT is read-only
            // Writing a BGxX/BGxY halfword reloads that BG's internal
            // affine reference point immediately.
            0x28 | 0x2A | 0x2C | 0x2E => {
                self.regs[Self::reg_index(offset)] = value;
                self.reload_ref(0);
            }
            0x38 | 0x3A | 0x3C | 0x3E => {
                self.regs[Self::reg_index(offset)] = value;
                self.reload_ref(1);
            }
            _ => self.regs[Self::reg_index(offset)] = value,
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

    /// Advance the PPU by `cycles`, rendering visible scanlines. Returns the
    /// interrupt bits to raise plus the VBlank/HBlank *events* (which drive
    /// DMA regardless of whether their interrupt is enabled).
    /// `vram`/`palette`/`oam` are borrowed from [`crate::memory::Memory`] for
    /// the duration of any rendering triggered here.
    pub fn tick(&mut self, cycles: u64, vram: &[u8], palette: &[u8], oam: &[u8]) -> PpuOutput {
        let mut out = PpuOutput::default();
        for _ in 0..cycles {
            self.tick_one(vram, palette, oam, &mut out);
        }
        out
    }

    fn tick_one(&mut self, vram: &[u8], palette: &[u8], oam: &[u8], out: &mut PpuOutput) {
        self.dot += 1;

        if self.dot == HDRAW_DOTS {
            // Entering HBlank: the line is now fully "drawn". HBlank DMA fires
            // only for visible lines.
            if self.vcount < SCREEN_H as u16 {
                self.render_scanline(self.vcount, vram, palette, oam);
                out.hblank_start = true;
                // Advance the affine reference points by (PB, PD) per line.
                for k in 0..2 {
                    self.ref_x[k] += self.bg_pb(k);
                    self.ref_y[k] += self.bg_pd(k);
                }
            }
            if self.dispstat() & 1 << 4 != 0 {
                out.irqs |= irq::HBLANK;
            }
        }

        if self.dot >= DOTS_PER_LINE {
            self.dot = 0;
            self.vcount += 1;
            if self.vcount >= TOTAL_LINES {
                self.vcount = 0;
                // Reload the affine reference points at the start of the frame.
                self.reload_ref(0);
                self.reload_ref(1);
            }
            if self.vcount == SCREEN_H as u16 {
                self.frame_ready = true;
                out.vblank_start = true;
                if self.dispstat() & 1 << 3 != 0 {
                    out.irqs |= irq::VBLANK;
                }
            }
            if self.vcount == self.dispstat() >> 8 && self.dispstat() & 1 << 5 != 0 {
                out.irqs |= irq::VCOUNT;
            }
        }
    }

    // ---- rendering ----

    /// How background `bg` behaves in the given tiled mode (0–2).
    fn bg_kind(mode: u16, bg: usize) -> Option<BgKind> {
        match (mode, bg) {
            (0, 0..=3) => Some(BgKind::Text),
            (1, 0 | 1) => Some(BgKind::Text),
            (1, 2) => Some(BgKind::Affine),
            (2, 2 | 3) => Some(BgKind::Affine),
            _ => None,
        }
    }

    fn render_scanline(&mut self, ly: u16, vram: &[u8], palette: &[u8], oam: &[u8]) {
        let base = ly as usize * SCREEN_W;

        // Forced blank (DISPCNT bit 7) outputs white.
        if self.dispcnt() & 1 << 7 != 0 {
            self.framebuffer[base..base + SCREEN_W].fill(0x7FFF);
            return;
        }

        // Each layer is rendered independently into its own buffer, then the
        // pixels are composited with window masking and the color special
        // effects (alpha blend / brighten / darken).
        let mut bg: [Layer; 4] = std::array::from_fn(|_| Layer::new());
        let mut bg_active = [false; 4];
        let mut obj = ObjLayer::new();

        let mode = self.dispcnt() & 0x7;
        if mode <= 2 {
            for i in 0..4 {
                if self.dispcnt() & (1 << (8 + i)) == 0 {
                    continue;
                }
                match Self::bg_kind(mode, i) {
                    Some(BgKind::Text) => {
                        bg_active[i] = true;
                        self.render_text_bg_line(i, ly, vram, palette, &mut bg[i]);
                    }
                    Some(BgKind::Affine) => {
                        bg_active[i] = true;
                        self.render_affine_bg_line(i, vram, palette, &mut bg[i]);
                    }
                    None => {}
                }
            }
        } else if self.dispcnt() & 1 << 10 != 0 {
            bg_active[2] = true;
            self.render_bitmap_line(mode, ly, vram, palette, &mut bg[2]);
        }

        if self.dispcnt() & 1 << 12 != 0 {
            self.render_sprites_line(ly, vram, palette, oam, &mut obj);
        }

        self.composite(ly, palette, &bg, &bg_active, &obj, base);
    }

    /// Composite the rendered layers into the framebuffer, applying window
    /// masks and the color special effects.
    #[allow(clippy::too_many_arguments)]
    fn composite(
        &mut self,
        ly: u16,
        palette: &[u8],
        bg: &[Layer; 4],
        bg_active: &[bool; 4],
        obj: &ObjLayer,
        base: usize,
    ) {
        let backdrop = read_u16(palette, 0);
        let bg_prio = [
            self.bgcnt(0) & 3,
            self.bgcnt(1) & 3,
            self.bgcnt(2) & 3,
            self.bgcnt(3) & 3,
        ];
        let bldcnt = self.regs[0x28];
        let bldalpha = self.regs[0x29];
        let bldy = self.regs[0x2A];
        let sfx_mode = (bldcnt >> 6) & 3;
        let eva = (bldalpha & 0x1F).min(16) as u32;
        let evb = ((bldalpha >> 8) & 0x1F).min(16) as u32;
        let evy = (bldy & 0x1F).min(16) as u32;
        let windows_on = self.dispcnt() & 0xE000 != 0;

        // Sort key: lower is more in front. OBJ beats a BG of equal priority;
        // lower BG index beats higher; the backdrop is always last.
        const BACKDROP_KEY: u16 = u16::MAX;

        for x in 0..SCREEN_W {
            let mask = if windows_on {
                self.window_mask(x, ly, obj.window[x])
            } else {
                0x3F
            };

            // Find the front-most (a) and second (b) enabled opaque layers.
            let mut a = (BACKDROP_KEY, backdrop, 5u8);
            let mut b = (BACKDROP_KEY, backdrop, 5u8);
            let mut consider = |key: u16, color: u16, layer: u8| {
                if key < a.0 {
                    b = a;
                    a = (key, color, layer);
                } else if key < b.0 {
                    b = (key, color, layer);
                }
            };
            for i in 0..4 {
                if bg_active[i] && bg[i].opaque[x] && mask & (1 << i) != 0 {
                    consider(bg_prio[i] * 8 + 1 + i as u16, bg[i].color[x], i as u8);
                }
            }
            if obj.opaque[x] && mask & (1 << 4) != 0 {
                consider(u16::from(obj.prio[x]) * 8, obj.color[x], 4);
            }

            let sfx = mask & (1 << 5) != 0;
            let is_t1 = |l: u8| bldcnt & (1 << l) != 0;
            let is_t2 = |l: u8| bldcnt & (1 << (8 + l)) != 0;
            let color = if a.2 == 4 && obj.semi[x] && is_t2(b.2) {
                // A semi-transparent OBJ alpha-blends with the layer behind it.
                alpha_blend(a.1, b.1, eva, evb)
            } else if sfx && sfx_mode == 1 && is_t1(a.2) && is_t2(b.2) {
                alpha_blend(a.1, b.1, eva, evb)
            } else if sfx && sfx_mode == 2 && is_t1(a.2) {
                brighten(a.1, evy)
            } else if sfx && sfx_mode == 3 && is_t1(a.2) {
                darken(a.1, evy)
            } else {
                a.1
            };
            self.framebuffer[base + x] = color;
        }
    }

    /// The 6-bit layer-enable mask (BG0–3, OBJ, special-effect) for a pixel,
    /// chosen by whichever window region contains it.
    fn window_mask(&self, x: usize, ly: u16, obj_win: bool) -> u16 {
        let d = self.dispcnt();
        if d & 1 << 13 != 0 && self.in_window(0, x, ly) {
            self.regs[0x24] & 0x3F // WININ, window 0
        } else if d & 1 << 14 != 0 && self.in_window(1, x, ly) {
            (self.regs[0x24] >> 8) & 0x3F // WININ, window 1
        } else if d & 1 << 15 != 0 && obj_win {
            (self.regs[0x25] >> 8) & 0x3F // WINOUT high, OBJ window
        } else {
            self.regs[0x25] & 0x3F // WINOUT low, outside all windows
        }
    }

    /// Whether (x, ly) is inside window `w` (0 or 1), honoring the X/Y wrap
    /// case where the "left" edge is greater than the "right".
    fn in_window(&self, w: usize, x: usize, ly: u16) -> bool {
        let h = self.regs[0x20 + w];
        let v = self.regs[0x22 + w];
        let (left, right) = ((h >> 8) as usize, (h & 0xFF) as usize);
        let (top, bottom) = ((v >> 8) as usize, (v & 0xFF) as usize);
        let ly = ly as usize;
        let inx = if left <= right {
            x >= left && x < right
        } else {
            x >= left || x < right
        };
        let iny = if top <= bottom {
            ly >= top && ly < bottom
        } else {
            ly >= top || ly < bottom
        };
        inx && iny
    }

    /// Render one text background into its own layer buffer (every pixel;
    /// index-0 pixels stay transparent).
    fn render_text_bg_line(
        &self,
        bg: usize,
        ly: u16,
        vram: &[u8],
        palette: &[u8],
        layer: &mut Layer,
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

        let (mosaic_x, mosaic_y) = self.bg_mosaic(bg);
        let mly = ly as usize - ly as usize % mosaic_y;
        let bgy = (mly + self.bgvofs(bg) as usize) % h_px;
        let ty = bgy / 8;
        let py = bgy % 8;

        for x in 0..SCREEN_W {
            let bgx = (x - x % mosaic_x + self.bghofs(bg) as usize) % w_px;
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
            layer.color[x] = read_u16(palette, pal_entry * 2);
            layer.opaque[x] = true;
        }
    }

    /// Render an affine background (BG2 or BG3) for the current scanline using
    /// its internal reference point and the PA/PC horizontal step. Affine BGs
    /// are always 8bpp with 1-byte map entries; BGxCNT bit 13 selects wrap vs
    /// transparent outside the map.
    fn render_affine_bg_line(&self, bg: usize, vram: &[u8], palette: &[u8], layer: &mut Layer) {
        let k = bg - 2; // BG2 -> 0, BG3 -> 1
        let cnt = self.bgcnt(bg);
        let char_base = (((cnt >> 2) & 0x3) as usize) * 0x4000;
        let screen_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let wrap = cnt & 1 << 13 != 0;
        // Sizes: 128, 256, 512, 1024 pixels square.
        let size_px = 128i32 << ((cnt >> 14) & 0x3);
        let tiles_wide = (size_px / 8) as usize;

        let pa = self.bg_pa(k);
        let pc = self.bg_pc(k);
        let (mosaic_x, _) = self.bg_mosaic(bg);

        for x in 0..SCREEN_W {
            let sx = (x - x % mosaic_x) as i32;
            // Texture coordinate in 20.8 fixed point, then to integer pixels.
            let mut tx = (self.ref_x[k] + sx * pa) >> 8;
            let mut ty = (self.ref_y[k] + sx * pc) >> 8;
            if wrap {
                tx = tx.rem_euclid(size_px);
                ty = ty.rem_euclid(size_px);
            } else if tx < 0 || tx >= size_px || ty < 0 || ty >= size_px {
                continue; // outside the map, and not wrapping → transparent
            }
            let (tx, ty) = (tx as usize, ty as usize);

            let tile = vram[screen_base + (ty / 8) * tiles_wide + tx / 8] as usize;
            let color_index = vram[char_base + tile * 64 + (ty % 8) * 8 + (tx % 8)] as usize;
            if color_index == 0 {
                continue;
            }
            layer.color[x] = read_u16(palette, color_index * 2);
            layer.opaque[x] = true;
        }
    }

    /// Render the BG2 bitmap for one scanline (modes 3–5).
    ///
    /// * Mode 3 — 240×160 direct 15-bit color, fully opaque.
    /// * Mode 4 — 240×160 8-bit paletted, double-buffered (DISPCNT bit 4
    ///   selects the frame); palette index 0 is transparent.
    /// * Mode 5 — 160×128 direct color, double-buffered; pixels outside the
    ///   smaller canvas show the backdrop.
    fn render_bitmap_line(
        &self,
        mode: u16,
        ly: u16,
        vram: &[u8],
        palette: &[u8],
        layer: &mut Layer,
    ) {
        let frame = if self.dispcnt() & 1 << 4 != 0 {
            0xA000
        } else {
            0
        };
        let ly = ly as usize;
        match mode {
            3 => {
                for x in 0..SCREEN_W {
                    layer.color[x] = read_u16(vram, (ly * SCREEN_W + x) * 2);
                    layer.opaque[x] = true;
                }
            }
            4 => {
                for x in 0..SCREEN_W {
                    let idx = vram[frame + ly * SCREEN_W + x] as usize;
                    if idx == 0 {
                        continue; // transparent
                    }
                    layer.color[x] = read_u16(palette, idx * 2);
                    layer.opaque[x] = true;
                }
            }
            _ => {
                // Mode 5: 160×128 canvas.
                if ly < 128 {
                    for x in 0..160 {
                        layer.color[x] = read_u16(vram, frame + (ly * 160 + x) * 2);
                        layer.opaque[x] = true;
                    }
                }
            }
        }
    }

    /// Render the sprite (OBJ) layer for one scanline over the background.
    ///
    /// Sprites are processed in OAM order so that a lower-index sprite wins an
    /// overlap; each sprite's pixel replaces the background pixel only when its
    /// priority is at least as high (`obj_prio <= bg_prio`). Regular and affine
    /// (rotation/scaling, incl. double-size) sprites, with OBJ mosaic. The OBJ
    /// window mode is Phase 5 part B.
    fn render_sprites_line(
        &self,
        ly: u16,
        vram: &[u8],
        palette: &[u8],
        oam: &[u8],
        obj: &mut ObjLayer,
    ) {
        let mapping_1d = self.dispcnt() & 1 << 6 != 0;
        let (mos_x, mos_y) = self.obj_mosaic();

        for i in 0..128 {
            let attr0 = read_u16(oam, i * 8);
            let attr1 = read_u16(oam, i * 8 + 2);
            let attr2 = read_u16(oam, i * 8 + 4);

            let affine = attr0 & 1 << 8 != 0;
            // Bit 9 disables a regular sprite, or doubles an affine one.
            if !affine && attr0 & 1 << 9 != 0 {
                continue;
            }
            // OBJ mode: 1 = semi-transparent (alpha blend), 2 = OBJ window mask.
            let obj_mode = (attr0 >> 10) & 0x3;
            let is_window = obj_mode == 2;
            let semi = obj_mode == 1;

            let shape = (attr0 >> 14) & 0x3;
            let size = (attr1 >> 14) & 0x3;
            let (w, h) = OBJ_SIZES[shape as usize][size as usize];
            let color8 = attr0 & 1 << 13 != 0;
            let mosaic = attr0 & 1 << 12 != 0;
            let tile_base = (attr2 & 0x3FF) as usize;
            let obj_prio = ((attr2 >> 10) & 0x3) as u8;
            let pal_bank = ((attr2 >> 12) & 0xF) as usize;

            // The on-screen bounding box; affine double-size doubles it.
            let double = affine && attr0 & 1 << 9 != 0;
            let (bw, bh) = if double { (w * 2, h * 2) } else { (w, h) };

            // Row within the bounding box (vertical position wraps at 256).
            let y = (attr0 & 0xFF) as usize;
            let by = (ly as usize + 256 - y) % 256;
            if by >= bh {
                continue;
            }
            let x = (attr1 & 0x1FF) as usize;

            // Per-column texture coordinate: an affine 2×2 transform about the
            // box centre, or a straight (optionally flipped) mapping.
            let (pa, pb, pc, pd) = if affine {
                let g = ((attr1 >> 9) & 0x1F) as usize * 0x20;
                (
                    read_i16(oam, g + 0x06),
                    read_i16(oam, g + 0x0E),
                    read_i16(oam, g + 0x16),
                    read_i16(oam, g + 0x1E),
                )
            } else {
                (0, 0, 0, 0)
            };
            let hflip = !affine && attr1 & 1 << 12 != 0;
            let vflip = !affine && attr1 & 1 << 13 != 0;

            for cx in 0..bw {
                let sx = (x + cx) & 0x1FF;
                // A color pixel is blocked by a lower-index sprite; an OBJ-window
                // pixel only needs to still be uncovered as a window.
                if sx >= SCREEN_W || (!is_window && obj.opaque[sx]) {
                    continue;
                }
                let (tx, ty) = if affine {
                    let ix = cx as i32 - bw as i32 / 2;
                    let iy = by as i32 - bh as i32 / 2;
                    let tx = ((pa * ix + pb * iy) >> 8) + w as i32 / 2;
                    let ty = ((pc * ix + pd * iy) >> 8) + h as i32 / 2;
                    if tx < 0 || tx >= w as i32 || ty < 0 || ty >= h as i32 {
                        continue; // outside the source sprite
                    }
                    (tx as usize, ty as usize)
                } else {
                    let tx = if hflip { w - 1 - cx } else { cx };
                    let ty = if vflip { h - 1 - by } else { by };
                    (tx, ty)
                };

                // Mosaic reduces effective resolution before sampling.
                let (tx, ty) = if mosaic {
                    (tx - tx % mos_x, ty - ty % mos_y)
                } else {
                    (tx, ty)
                };

                if let Some((ci, pal_base)) =
                    Self::sample_obj(vram, tile_base, tx, ty, w, color8, pal_bank, mapping_1d)
                {
                    if is_window {
                        obj.window[sx] = true; // masks compositing, draws nothing
                    } else {
                        obj.opaque[sx] = true;
                        obj.color[sx] = read_u16(palette, pal_base + ci * 2);
                        obj.prio[sx] = obj_prio;
                        obj.semi[sx] = semi;
                    }
                }
            }
        }
    }

    /// Sample one texel of an OBJ tile; returns `(color_index, palette byte
    /// base)` or `None` if transparent.
    #[allow(clippy::too_many_arguments)]
    fn sample_obj(
        vram: &[u8],
        tile_base: usize,
        tx: usize,
        ty: usize,
        w: usize,
        color8: bool,
        pal_bank: usize,
        mapping_1d: bool,
    ) -> Option<(usize, usize)> {
        let step = if color8 { 2 } else { 1 };
        let tile_index = if mapping_1d {
            tile_base + (ty / 8 * (w / 8) + tx / 8) * step
        } else {
            tile_base + ty / 8 * 32 + tx / 8 * step
        };
        let (px, py) = (tx & 7, ty & 7);
        let (color_index, pal_base) = if color8 {
            let off = tile_index * 32 + py * 8 + px;
            (vram[0x10000 + (off & 0x7FFF)] as usize, 0x200)
        } else {
            let off = tile_index * 32 + py * 4 + px / 2;
            let byte = vram[0x10000 + (off & 0x7FFF)];
            let nibble = if px & 1 == 0 { byte & 0xF } else { byte >> 4 };
            (nibble as usize, 0x200 + pal_bank * 32)
        };
        if color_index == 0 {
            None
        } else {
            Some((color_index, pal_base))
        }
    }
}

/// OBJ pixel dimensions indexed by `[shape][size]`. Shape 3 is prohibited on
/// hardware; we map it to square so it never panics.
const OBJ_SIZES: [[(usize, usize); 4]; 4] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)], // square
    [(16, 8), (32, 8), (32, 16), (64, 32)], // horizontal
    [(8, 16), (8, 32), (16, 32), (32, 64)], // vertical
    [(8, 8), (16, 16), (32, 32), (64, 64)], // prohibited → square
];

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

fn read_u16(data: &[u8], addr: usize) -> u16 {
    u16::from(data[addr]) | u16::from(data[addr + 1]) << 8
}

/// Read a signed 16-bit value (affine parameters) as i32.
fn read_i16(data: &[u8], addr: usize) -> i32 {
    read_u16(data, addr) as i16 as i32
}

/// Sign-extend a 28-bit value (affine reference registers) to i32.
fn sign_extend_28(value: u32) -> i32 {
    ((value << 4) as i32) >> 4
}

fn channels(color: u16) -> (u32, u32, u32) {
    (
        u32::from(color & 0x1F),
        u32::from((color >> 5) & 0x1F),
        u32::from((color >> 10) & 0x1F),
    )
}

fn pack(r: u32, g: u32, b: u32) -> u16 {
    (r.min(31) | g.min(31) << 5 | b.min(31) << 10) as u16
}

/// Alpha-blend `top` over `bottom` with the BLDALPHA coefficients (eva/evb are
/// 0..16 = 0/16..16/16).
fn alpha_blend(top: u16, bottom: u16, eva: u32, evb: u32) -> u16 {
    let (tr, tg, tb) = channels(top);
    let (br, bg, bb) = channels(bottom);
    pack(
        (tr * eva + br * evb) >> 4,
        (tg * eva + bg * evb) >> 4,
        (tb * eva + bb * evb) >> 4,
    )
}

/// Brightness increase toward white by `evy`/16 (BLDY, effect mode 2).
fn brighten(color: u16, evy: u32) -> u16 {
    let (r, g, b) = channels(color);
    pack(
        r + (((31 - r) * evy) >> 4),
        g + (((31 - g) * evy) >> 4),
        b + (((31 - b) * evy) >> 4),
    )
}

/// Brightness decrease toward black by `evy`/16 (BLDY, effect mode 3).
fn darken(color: u16, evy: u32) -> u16 {
    let (r, g, b) = channels(color);
    pack(
        r - ((r * evy) >> 4),
        g - ((g * evy) >> 4),
        b - ((b * evy) >> 4),
    )
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
