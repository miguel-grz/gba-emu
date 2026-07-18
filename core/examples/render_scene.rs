//! Headless PPU demo: build a tiled scene by hand, render one frame, and dump
//! it to a 24-bit BMP you can open in any image viewer.
//!
//!   cargo run --example render_scene -- scene.bmp
//!
//! This exercises the Phase-3 text renderer end to end — multiple tiles,
//! a 16-color palette, screen-map addressing, sub-tile patterns, and
//! transparency (index 0 shows the backdrop) — without needing a game ROM.

use gba_core::memory::Bus;
use gba_core::ppu::{bgr555_to_rgb888, SCREEN_H, SCREEN_W};
use gba_core::Memory;
use std::io::Write;

const PALETTE: u32 = 0x0500_0000;
const VRAM: u32 = 0x0600_0000;
const SCREEN_BASE_BLOCK: u32 = 1;

fn set_palette(m: &mut Memory, index: u32, color: u16) {
    m.write16(PALETTE + index * 2, color);
}

/// Write a solid 4bpp tile (all pixels = `idx`).
fn solid_tile(m: &mut Memory, tile: u32, idx: u8) {
    let packed = u16::from(idx | idx << 4) * 0x0101;
    for i in 0..16 {
        m.write16(VRAM + tile * 32 + i * 2, packed);
    }
}

/// Write a 4bpp tile with a diagonal of `idx` over a transparent (index 0)
/// field, to show sub-tile detail and transparency.
fn diagonal_tile(m: &mut Memory, tile: u32, idx: u8) {
    for y in 0..8u32 {
        for x in 0..8u32 {
            let v = if x == y || x + y == 7 { idx } else { 0 };
            let addr = VRAM + tile * 32 + y * 4 + x / 2;
            let byte = m.read8(addr);
            let byte = if x & 1 == 0 {
                (byte & 0xF0) | v
            } else {
                (byte & 0x0F) | (v << 4)
            };
            m.write8(addr, byte);
        }
    }
}

fn set_map(m: &mut Memory, tx: u32, ty: u32, tile: u16) {
    let addr = VRAM + SCREEN_BASE_BLOCK * 0x800 + (ty * 32 + tx) * 2;
    m.write16(addr, tile);
}

fn build_scene(m: &mut Memory) {
    // Palette: 0 = backdrop, then a small themed set.
    set_palette(m, 0, 0x1000); // dark blue backdrop
    set_palette(m, 1, 0x0208); // teal
    set_palette(m, 2, 0x029F); // orange
    set_palette(m, 3, 0x7FFF); // white

    solid_tile(m, 1, 1);
    solid_tile(m, 2, 2);
    diagonal_tile(m, 3, 3);

    // 30×20 visible tiles. Checkerboard of tiles 1/2, with a centered
    // rectangle of the diagonal tile 3.
    for ty in 0..20u32 {
        for tx in 0..30u32 {
            let checker = if (tx + ty) % 2 == 0 { 1 } else { 2 };
            let inside = (10..20).contains(&tx) && (7..13).contains(&ty);
            set_map(m, tx, ty, if inside { 3 } else { checker });
        }
    }

    m.write16(0x0400_0008, (SCREEN_BASE_BLOCK << 8) as u16); // BG0CNT
    m.write16(0x0400_0000, 1 << 8); // DISPCNT: mode 0, BG0 on
}

fn write_bmp(path: &str, fb: &[u16]) -> std::io::Result<()> {
    let (w, h) = (SCREEN_W, SCREEN_H);
    let row = w * 3;
    let pixel_bytes = row * h;
    let file_size = 54 + pixel_bytes;
    let mut out = Vec::with_capacity(file_size);

    // BITMAPFILEHEADER
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]); // no compression, sizes/resolution zero

    // Pixel data, bottom-up, BGR order (row is a multiple of 4 for 240px).
    for y in (0..h).rev() {
        for x in 0..w {
            let (r, g, b) = bgr555_to_rgb888(fb[y * w + x]);
            out.push(b);
            out.push(g);
            out.push(r);
        }
    }

    let mut f = std::fs::File::create(path)?;
    f.write_all(&out)
}

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scene.bmp".to_string());

    let mut m = Memory::new(vec![0; 0x100]).expect("rom");
    build_scene(&mut m);
    // Advance one full field so every visible scanline renders.
    m.tick(160 * 1232);

    write_bmp(&path, m.framebuffer())?;
    println!("Wrote {}×{} frame to {path}", SCREEN_W, SCREEN_H);
    Ok(())
}
