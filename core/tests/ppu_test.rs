//! Phase 3 tests: PPU registers, display timing/interrupts, and text
//! (tiled) background rendering for modes 0–1.

use gba_core::io::irq;
use gba_core::memory::Bus;
use gba_core::ppu::{bgr555_to_rgb888, SCREEN_W};
use gba_core::{Gba, Memory};

// I/O addresses.
const DISPCNT: u32 = 0x0400_0000;
const DISPSTAT: u32 = 0x0400_0004;
const VCOUNT: u32 = 0x0400_0006;
const BG0CNT: u32 = 0x0400_0008;
const BG1CNT: u32 = 0x0400_000A;
const BG0HOFS: u32 = 0x0400_0010;
const IE: u32 = 0x0400_0200;
const IF: u32 = 0x0400_0202;

const PALETTE: u32 = 0x0500_0000;
const VRAM: u32 = 0x0600_0000;

const LINE_CYCLES: u64 = 1232;

fn blank() -> Memory {
    Memory::new(vec![0; 0x100]).unwrap()
}

/// Write a 4bpp tile whose every pixel uses palette index `idx`.
fn fill_tile_4bpp(m: &mut Memory, char_base: u32, tile: u32, idx: u8) {
    // Both nibbles of the byte, and both bytes of the halfword, are `idx`.
    let packed = u16::from(idx | idx << 4) * 0x0101;
    for i in 0..16 {
        m.write16(VRAM + char_base + tile * 32 + i * 2, packed);
    }
}

// ------------------------------------------------------------- registers

#[test]
fn ppu_registers_read_back() {
    let mut m = blank();
    m.write16(DISPCNT, 0x0140);
    m.write16(BG0CNT, 0x1E83);
    m.write16(BG0HOFS, 0x01FF);
    assert_eq!(m.read16(DISPCNT), 0x0140);
    assert_eq!(m.read16(BG0CNT), 0x1E83);
    assert_eq!(m.read16(BG0HOFS), 0x01FF);
}

#[test]
fn dispstat_status_bits_are_read_only() {
    let mut m = blank();
    // Try to set the (read-only) VBlank/HBlank status bits; they must not
    // stick, but the enable bits (3–5) and VCount setting (8–15) must.
    m.write16(DISPSTAT, 0xFFFF);
    let v = m.read16(DISPSTAT);
    assert_eq!(v & 0x0007, 0, "status bits not settable at line 0/dot 0");
    assert_eq!(v & 0x0038, 0x0038, "irq-enable bits stored");
    assert_eq!(v >> 8, 0xFF, "vcount setting stored");
}

// --------------------------------------------------------- display timing

#[test]
fn vcount_advances_and_vblank_flag_sets() {
    let mut m = blank();
    assert_eq!(m.read16(VCOUNT), 0);
    // Advance 160 scanlines to reach the start of VBlank.
    m.tick(160 * LINE_CYCLES);
    assert_eq!(m.read16(VCOUNT), 160);
    assert_eq!(m.read16(DISPSTAT) & 1, 1, "VBlank flag set at line 160");
    // ...and it clears again once the frame wraps back to line 0.
    m.tick(68 * LINE_CYCLES);
    assert_eq!(m.read16(VCOUNT), 0);
    assert_eq!(m.read16(DISPSTAT) & 1, 0);
}

#[test]
fn vblank_interrupt_raised_when_enabled() {
    let mut m = blank();
    m.write16(DISPSTAT, 1 << 3); // enable VBlank IRQ
    m.tick(160 * LINE_CYCLES);
    assert_eq!(m.read16(IF) & irq::VBLANK, irq::VBLANK);
}

#[test]
fn vblank_interrupt_suppressed_when_disabled() {
    let mut m = blank();
    m.tick(160 * LINE_CYCLES); // enable bit clear
    assert_eq!(m.read16(IF) & irq::VBLANK, 0);
}

#[test]
fn vcount_match_interrupt() {
    let mut m = blank();
    // Enable VCount IRQ (bit 5) with target line 80 (bits 8–15).
    m.write16(DISPSTAT, (1 << 5) | (80 << 8));
    m.tick(80 * LINE_CYCLES);
    assert_eq!(m.read16(VCOUNT), 80);
    assert_eq!(m.read16(IF) & irq::VCOUNT, irq::VCOUNT);
}

// ----------------------------------------------------- text BG rendering

/// Render scanline 0 with the given setup and return its 240-pixel row.
fn render_line0(setup: impl FnOnce(&mut Memory)) -> Vec<u16> {
    let mut m = blank();
    setup(&mut m);
    m.tick(960); // reach HBlank of line 0, which triggers its render
    m.framebuffer()[0..SCREEN_W].to_vec()
}

#[test]
fn text_bg_4bpp_solid_fill() {
    let color = 0x03E0; // green in BGR555
    let line = render_line0(|m| {
        m.write16(PALETTE + 2, color); // palette index 1
        fill_tile_4bpp(m, 0, 0, 1); // tile 0 = all index 1
        m.write16(BG0CNT, 1 << 8); // screen base block 1, char base 0, 4bpp
        m.write16(DISPCNT, 1 << 8); // mode 0, BG0 enabled
    });
    assert!(
        line.iter().all(|&px| px == color),
        "whole line is the tile color"
    );
}

#[test]
fn text_bg_8bpp_solid_fill() {
    let color = 0x7C00; // blue
    let line = render_line0(|m| {
        m.write16(PALETTE + 5 * 2, color); // 256-color palette index 5
        for i in 0..32 {
            m.write16(VRAM + i * 2, 0x0505); // tile 0, 64 bytes all = 5
        }
        m.write16(BG0CNT, (1 << 8) | (1 << 7)); // screen base 1, 8bpp
        m.write16(DISPCNT, 1 << 8);
    });
    assert!(line.iter().all(|&px| px == color));
}

#[test]
fn text_bg_backdrop_shows_through_transparent_pixels() {
    let backdrop = 0x001F; // red
    let line = render_line0(|m| {
        m.write16(PALETTE, backdrop); // palette index 0 = backdrop
                                      // BG0 enabled but its tiles are all index 0 (transparent).
        m.write16(BG0CNT, 1 << 8);
        m.write16(DISPCNT, 1 << 8);
    });
    assert!(line.iter().all(|&px| px == backdrop));
}

#[test]
fn text_bg_horizontal_scroll() {
    // Column 0 uses index 2, the rest index 1. Scrolling right by 1 pixel
    // should push column 0's color off-screen at x=0.
    let c1 = 0x0200;
    let c2 = 0x0300;
    let setup = |hofs: u16| {
        move |m: &mut Memory| {
            m.write16(PALETTE + 2, c1);
            m.write16(PALETTE + 4, c2);
            // Tile 0: pixel (0,0) = index 2, pixels (1..7,0) = index 1.
            m.write16(VRAM, 0x0012); // byte0: px0=2, px1=1
            for i in 1..16 {
                m.write16(VRAM + i * 2, 0x0011);
            }
            m.write16(BG0CNT, 1 << 8);
            m.write16(DISPCNT, 1 << 8);
            m.write16(BG0HOFS, hofs);
        }
    };
    assert_eq!(
        render_line0(setup(0))[0],
        c2,
        "unscrolled: x0 shows index 2"
    );
    assert_eq!(
        render_line0(setup(1))[0],
        c1,
        "scrolled by 1: index 1 now at x0"
    );
}

#[test]
fn text_bg_priority_orders_layers() {
    let front = 0x001F;
    let back = 0x7C00;
    let line = render_line0(|m| {
        m.write16(PALETTE + 2, back); // BG0 uses index 1
        m.write16(PALETTE + 4, front); // BG1 uses index 2
                                       // BG0: char base 0, tile 0 = index 1, screen base block 1, priority 1.
        fill_tile_4bpp(m, 0, 0, 1);
        m.write16(BG0CNT, (1 << 8) | 1);
        // BG1: char base 0x4000, tile 0 = index 2, screen base block 2, prio 0.
        fill_tile_4bpp(m, 0x4000, 0, 2);
        m.write16(BG1CNT, (1 << 2) | (2 << 8));
        m.write16(DISPCNT, (1 << 8) | (1 << 9)); // BG0 + BG1
    });
    assert!(
        line.iter().all(|&px| px == front),
        "higher-priority BG1 wins"
    );
}

#[test]
fn forced_blank_outputs_white() {
    let line = render_line0(|m| {
        m.write16(DISPCNT, 1 << 7); // forced blank
    });
    assert!(line.iter().all(|&px| px == 0x7FFF));
}

// ---------------------------------------------------- frame loop + IRQ path

#[test]
fn run_frame_reaches_vblank_and_services_irq() {
    // A tiny ROM: enable VBlank IRQ + IME, then spin. When VBlank fires the
    // CPU should vector to the IRQ handler.
    let code: [u32; 6] = [
        0xE3A00301, // mov r0, #0x04000000
        0xE3A01008, // mov r1, #8            ; DISPSTAT VBlank-IRQ enable
        0xE1C010B4, // strh r1, [r0, #4]     ; DISPSTAT = 8
        0xE3A02001, // mov r2, #1
        0xE5802208, // str r2, [r0, #0x208]  ; IME = 1 (and IE below)
        0xEAFFFFFE, // b .                    ; spin
    ];
    let mut rom = Vec::new();
    for op in code {
        rom.extend_from_slice(&op.to_le_bytes());
    }
    let mut gba = Gba::new(rom).unwrap();
    gba.mem.write16(IE, irq::VBLANK); // enable the VBlank line
    gba.run_frame(2_000_000);
    // Entering VBlank set the flag and the IF bit.
    assert_eq!(gba.mem.read16(IF) & irq::VBLANK, irq::VBLANK);
    assert!(gba.mem.vcount() >= 160);
}

// -------------------------------------------------------------- sprites

const OAM: u32 = 0x0700_0000;
const OBJ_TILES: u32 = 0x0601_0000;
const OBJ_PAL: u32 = 0x0500_0200;

/// Solid 4bpp OBJ tile (all pixels = `idx`).
fn obj_tile_4bpp(m: &mut Memory, tile: u32, idx: u8) {
    let packed = u16::from(idx | idx << 4) * 0x0101;
    for i in 0..16 {
        m.write16(OBJ_TILES + tile * 32 + i * 2, packed);
    }
}

/// Write OBJ attributes for sprite `i`.
fn set_sprite(m: &mut Memory, i: u32, attr0: u16, attr1: u16, attr2: u16) {
    m.write16(OAM + i * 8, attr0);
    m.write16(OAM + i * 8 + 2, attr1);
    m.write16(OAM + i * 8 + 4, attr2);
}

#[test]
fn sprite_4bpp_renders_at_position() {
    let color = 0x03E0;
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 2, color); // OBJ palette bank 0, index 1
        obj_tile_4bpp(m, 1, 1);
        // 8×8 sprite at x=100, y=0, tile 1.
        set_sprite(m, 0, 0, 100, 1);
        m.write16(DISPCNT, 1 << 12); // OBJ enabled
    });
    assert!(line[100..108].iter().all(|&px| px == color));
    assert_eq!(line[99], 0, "nothing outside the sprite");
    assert_eq!(line[108], 0);
}

#[test]
fn sprite_8bpp_renders() {
    let color = 0x7C00;
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 5 * 2, color); // 256-color OBJ index 5
        for i in 0..32 {
            m.write16(OBJ_TILES + i * 2, 0x0505); // tile 0, 64 bytes all = 5
        }
        set_sprite(m, 0, 1 << 13, 50, 0); // attr0 bit13 = 8bpp
        m.write16(DISPCNT, 1 << 12);
    });
    assert!(line[50..58].iter().all(|&px| px == color));
}

#[test]
fn sprite_horizontal_flip() {
    let a = 0x0200;
    let b = 0x0300;
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 2, a); // index 1
        m.write16(OBJ_PAL + 4, b); // index 2
                                   // Tile 1: leftmost pixel index 2, the rest index 1.
        obj_tile_4bpp(m, 1, 1); // all index 1
        m.write16(OBJ_TILES + 32, 0x1112); // row 0: px0 = index 2
        set_sprite(m, 0, 0, (1 << 12) | 20, 1); // hflip, x=20
        m.write16(DISPCNT, 1 << 12);
    });
    // With H-flip, the index-2 pixel that was at column 0 is now at column 7.
    assert_eq!(line[27], b);
    assert_eq!(line[20], a);
}

#[test]
fn sprite_beats_same_priority_background() {
    let bg_color = 0x001F;
    let obj_color = 0x7C00;
    let mut m = blank();
    m.write16(PALETTE + 2, bg_color);
    fill_tile_4bpp(&mut m, 0, 0, 1);
    m.write16(BG0CNT, 1 << 8); // BG0 priority 0
    m.write16(OBJ_PAL + 2, obj_color);
    obj_tile_4bpp(&mut m, 1, 1);
    set_sprite(&mut m, 0, 0, 30, 1); // OBJ priority 0 (attr2 bits 10-11 = 0)
    m.write16(DISPCNT, (1 << 8) | (1 << 12)); // BG0 + OBJ
    m.tick(960);
    let line = &m.framebuffer()[0..SCREEN_W];
    assert_eq!(line[10], bg_color, "BG only, away from the sprite");
    assert_eq!(line[33], obj_color, "OBJ wins the same-priority tie");
}

#[test]
fn sprite_behind_higher_priority_background() {
    let bg_color = 0x001F;
    let obj_color = 0x7C00;
    let mut m = blank();
    m.write16(PALETTE + 2, bg_color);
    fill_tile_4bpp(&mut m, 0, 0, 1);
    m.write16(BG0CNT, 1 << 8); // BG0 priority 0
    m.write16(OBJ_PAL + 2, obj_color);
    obj_tile_4bpp(&mut m, 1, 1);
    set_sprite(&mut m, 0, 0, 30, (2 << 10) | 1); // OBJ priority 2
    m.write16(DISPCNT, (1 << 8) | (1 << 12));
    m.tick(960);
    let line = &m.framebuffer()[0..SCREEN_W];
    assert_eq!(line[33], bg_color, "higher-priority BG covers the sprite");
}

#[test]
fn lower_index_sprite_wins_overlap() {
    let c0 = 0x001F;
    let c1 = 0x7C00;
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 2, c0); // sprite 0 uses index 1
        m.write16(OBJ_PAL + 4, c1); // sprite 1 uses index 2
        obj_tile_4bpp(m, 1, 1);
        obj_tile_4bpp(m, 2, 2);
        set_sprite(m, 0, 0, 40, 1); // sprite 0 at x=40, tile 1
        set_sprite(m, 1, 0, 40, 2); // sprite 1 same spot, tile 2
        m.write16(DISPCNT, 1 << 12);
    });
    assert!(
        line[40..48].iter().all(|&px| px == c0),
        "lowest OAM index wins"
    );
}

#[test]
fn disabled_sprite_not_drawn() {
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 2, 0x7FFF);
        obj_tile_4bpp(m, 1, 1);
        set_sprite(m, 0, 1 << 9, 60, 1); // attr0 bit9 = disable
        m.write16(DISPCNT, 1 << 12);
    });
    assert!(line.iter().all(|&px| px == 0), "disabled sprite invisible");
}

#[test]
fn wide_sprite_shape_and_size() {
    // 32×8 sprite (shape = horizontal, size = 1) should span 32 px.
    let color = 0x03E0;
    let line = render_line0(|m| {
        m.write16(OBJ_PAL + 2, color);
        for t in 1..5 {
            obj_tile_4bpp(m, t, 1); // tiles 1..4 across the sprite
        }
        let shape_h = 1u16 << 14;
        let size_1 = 1u16 << 14;
        set_sprite(m, 0, shape_h, size_1, 1);
        m.write16(DISPCNT, 1 << 12);
    });
    assert!(line[0..32].iter().all(|&px| px == color), "32px wide");
    assert_eq!(line[32], 0);
}

// ------------------------------------------------------------ color helper

#[test]
fn color_conversion_extends_channels() {
    assert_eq!(bgr555_to_rgb888(0x001F), (255, 0, 0)); // max red
    assert_eq!(bgr555_to_rgb888(0x03E0), (0, 255, 0)); // max green
    assert_eq!(bgr555_to_rgb888(0x7C00), (0, 0, 255)); // max blue
    assert_eq!(bgr555_to_rgb888(0x0000), (0, 0, 0));
}
