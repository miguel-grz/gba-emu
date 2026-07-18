//! Load a GBA ROM, run it for a few frames, and dump the resulting frame to a
//! 24-bit BMP. Exercises the whole core (CPU → DMA/timers → PPU) on real code.
//!
//!   cargo run --example render_rom -- <rom.gba> [out.bmp] [frames]

use gba_core::ppu::{bgr555_to_rgb888, SCREEN_H, SCREEN_W};
use gba_core::Gba;
use std::io::Write;

fn write_bmp(path: &str, fb: &[u16]) -> std::io::Result<()> {
    let (w, h) = (SCREEN_W, SCREEN_H);
    let pixel_bytes = w * 3 * h;
    let mut out = Vec::with_capacity(54 + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let (r, g, b) = bgr555_to_rgb888(fb[y * w + x]);
            out.push(b);
            out.push(g);
            out.push(r);
        }
    }
    std::fs::File::create(path)?.write_all(&out)
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let rom_path = args
        .next()
        .expect("usage: render_rom <rom.gba> [out.bmp] [frames]");
    let out_path = args.next().unwrap_or_else(|| "rom.bmp".to_string());
    let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);

    let rom = std::fs::read(&rom_path)?;
    let mut gba = Gba::new(rom).expect("failed to load ROM");
    for _ in 0..frames {
        gba.run_frame(2_000_000);
    }

    write_bmp(&out_path, gba.framebuffer())?;
    println!(
        "Ran {frames} frames of {rom_path}; wrote frame to {out_path} (vcount={})",
        gba.mem.vcount()
    );
    Ok(())
}
