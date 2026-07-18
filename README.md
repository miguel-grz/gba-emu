# gba-emu

An open-source Game Boy Advance emulator with the eventual goal of a genuinely
polished, modern UI.

- **Core**: Rust (`core/`, crate `gba-core`) — zero UI dependencies
- **Frontend** (later phase): Tauri + React + TypeScript
- **Audio** (later phase): `cpal`
- **Hardware reference**: [GBATEK](https://problemkaputt.de/gbatek.htm)

## Status

| Phase | Component | Status |
|-------|-----------|--------|
| 1 | ARM7TDMI CPU (ARM + Thumb, banked registers, pipeline, exceptions) | ✅ done |
| 2 | Memory/bus (I/O registers, waitstate timing, open bus, BIOS HLE + LLE) | ✅ done |
| 3 | PPU tiled modes 0–2 (text backgrounds, scroll, priority, display IRQs) | ✅ done |
| 4 | Sprites (OBJ — regular, 4/8bpp, flip, 1D/2D mapping, priority) | ✅ done |
| 6 | DMA (4 channels) + timers (4, cascade) + interrupt wiring | ✅ done |
| 5 | Bitmap modes, affine BGs/sprites, mosaic, windows, alpha blending | ✅ done |
| 7 | APU — 4 PSG channels ✅; Direct Sound (DMA PCM) pending | 🚧 partial |
| 8 | Tauri shell + UI | — |

## Layout

```
core/
├── src/
│   ├── cpu/
│   │   ├── mod.rs      # register file, banking, CPSR/SPSR, pipeline, shifter
│   │   ├── arm.rs      # 32-bit ARM instruction set
│   │   └── thumb.rs    # 16-bit Thumb instruction set
│   ├── memory.rs       # Bus trait + full memory map, timing, open bus
│   ├── io.rs           # I/O register file (interrupt controller, WAITCNT, …)
│   ├── ppu.rs          # PPU: text backgrounds + sprites + display timing/IRQs
│   ├── dma.rs          # 4-channel DMA controller
│   ├── timers.rs       # 4 hardware timers (prescaler + cascade)
│   ├── apu.rs          # audio: 4 PSG channels, mixing, sample buffer
│   ├── timing.rs       # waitstate / S-N cycle model
│   ├── bios.rs         # BIOS SWI handling (HLE routines + LLE fallback)
│   ├── system.rs       # Gba: CPU + memory + PPU + DMA + timers, run_frame loop
│   └── lib.rs
├── examples/
│   └── render_scene.rs # headless PPU demo → 24-bit BMP
└── tests/
    ├── cpu_test.rs     # hand-assembled instruction tests + headless ROM harness
    ├── bus_test.rs     # I/O, timing, BIOS HLE, open bus, interrupts/halt
    ├── ppu_test.rs     # PPU registers, display timing/IRQs, text + sprite rendering
    ├── system_test.rs  # DMA channels, timers, and their interrupts
    └── apu_test.rs     # PSG channels, mixing, and the sample stream
```

## Seeing the PPU work

No game ROM needed — this renders a hand-built tiled scene to a BMP:

```sh
cargo run --example render_scene -- scene.bmp
```

It exercises the renderer end to end — multiple tiles, palette, screen-map
addressing, sub-tile detail, transparency over the backdrop, and a 16×16
sprite (bordered, priority over the background) drawn from OAM.

To run a real ROM and dump its first frame:

```sh
cargo run --example render_rom -- path/to/rom.gba out.bmp
```

The core boots public-domain homebrew (e.g. jsmolka's `hello.gba` renders
"Hello world!" in mode 4) with no BIOS image required.

## Building and testing

```sh
cargo test
```

The suite is self-contained (60+ tests). The CPU tests cover ALU flags,
barrel-shifter edge cases, pipeline-visible PC offsets, misaligned load
rotation, LDM/STM quirks, mode banking, exceptions and ARM↔Thumb interworking.
The bus tests cover the I/O registers, waitstate timing, the BIOS HLE routines,
cartridge open bus, the palette/VRAM/OAM 8-bit-write quirks, and interrupt/halt
behavior.

### Test ROMs (optional, recommended)

ROMs are not redistributed here. Drop these into `core/tests/roms/` and the
harness picks them up automatically (tests skip with a notice otherwise):

| File | Source | What it gives us |
|------|--------|------------------|
| `arm.gba`, `thumb.gba` | [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) | Real pass/fail assertion: on failure the ROM parks with the failing test number in `r12` |
| `armwrestler.gba` | ARMWrestler (mGBA fork: [mgba.io](https://mgba.io)) | Smoke test only until the PPU exists — results are rendered on screen |

No BIOS image is required (the core boots ROMs via `Cpu::skip_bios`, the
post-BIOS state). ROMs that call BIOS SWI routines need a real BIOS loaded
via `Memory::load_bios`.

## Accuracy notes (Phase 1 trade-offs)

- **Pipeline**: modeled as a two-slot fetch queue — every architecturally
  visible effect is correct (PC reads +8/+4, stores of PC +12, flush on
  branch, stale opcodes for self-modifying code).
- **ARM7 quirks implemented**: misaligned LDR/LDRH rotation, LDRSH→LDRSB at
  odd addresses, LDM/STM empty-list and base-in-list behavior, user-bank
  transfers (S bit), `MOV pc`/`POP {pc}` non-interworking on ARMv4T.
- **Deliberately unpredictable-as-benign**: MSR cannot flip the T bit,
  invalid mode writes keep the old mode, ARMv5-only encodings (BLX, LDRD)
  take the undefined trap or act as no-ops per hardware.

## Accuracy notes (Phase 2 trade-offs)

- **Cycle counts** are now driven by the bus: memory-access cycles come from
  the [`timing`](core/src/timing.rs) waitstate model, and instruction handlers
  add only internal (I-)cycles. Sequential vs non-sequential is inferred from
  address adjacency. The cartridge **prefetch buffer** (WAITCNT bit 14) is not
  modeled yet, so ROM-heavy code runs a few percent *slow* (a conservative
  error); it lands with the Phase 6 scheduler where prefetch stalls interact
  with DMA.
- **BIOS**: HLE by default (no copyrighted image needed); load a real BIOS via
  `Memory::load_bios` for LLE. HLE `Div`/`Sqrt`/`CpuSet`/`CpuFastSet`/
  `RegisterRamReset` are exact; `ArcTan`/`ArcTan2` use the mathematically
  correct result rather than the BIOS polynomial; `IntrWait`/`VBlankIntrWait`
  halt until an interrupt (their per-source bookkeeping completes when
  interrupt sources exist); decompression/affine-set/BitUnpack are not yet
  implemented (use LLE if a game needs them).
- **8-bit-write quirks**: palette and BG-VRAM byte writes duplicate across the
  halfword; OAM ignores byte writes. The BG/OBJ VRAM boundary is fixed at
  `0x14000` for now and becomes mode-dependent with the Phase 3 PPU.

## Accuracy notes (Phase 5 trade-offs)

- The PPU now covers the full tiled/bitmap feature set: bitmap modes 3–5;
  affine BG2/BG3 with per-scanline reference-point updates and wrap/transparent
  overflow; affine sprites including double-size; BG and OBJ mosaic; windows
  (WIN0/WIN1/OBJ window); and the color special effects — alpha blending,
  brighten, darken, and OBJ semi-transparency.
- **Compositing** renders each layer into its own scanline buffer, then per
  pixel picks the front-most and second layers (after window masking) and
  applies the blend. This is clearer than an in-place painter and is what makes
  two-target blending possible; the cost is five full-width buffers per line,
  which a later optimization pass can trim if needed.
- Affine-sprite mosaic is applied in texture space (a close approximation of
  hardware's screen-space mosaic).

## Accuracy notes (Phase 7 trade-offs)

- **Done (part A)**: the four PSG channels — two square (channel 1 with
  frequency sweep), the wave channel, and the noise LFSR — each with volume
  envelope and length counter, clocked by a 512 Hz frame sequencer, mixed to
  stereo per SOUNDCNT_L/H and resampled to 32768 Hz into a sample buffer the
  frontend will drain into `cpal`.
- **Pending (part B)**: Direct Sound — the two DMA-fed 8-bit PCM FIFOs clocked
  by a timer overflow. This is what most commercial games actually use for
  music, and it wires into the "special" DMA start-timing left from Phase 6.
- Mixing levels are a reasonable approximation, not calibrated against
  hardware's exact DAC/SOUNDBIAS response; there is no audio output to
  calibrate against until the Phase 8 frontend.
- **Still stubbed**: cartridge SRAM/Flash/EEPROM saves read as 0, and BIOS
  read-protection is not enforced. Neither affects CPU test ROMs.

## Accuracy notes (Phase 3 trade-offs)

- **Scope**: text (tiled) backgrounds of modes 0–2 only. Mode 1's affine BG2
  and mode 2's affine backgrounds render nothing yet; sprites, the bitmap
  modes 3–5, windows, mosaic and blending are Phases 4–5. The register storage
  for all of them already exists, so reads/writes behave — only rendering is
  absent.
- **Renderer**: scanline-based — each visible line is drawn at the start of its
  HBlank, so mid-frame register changes take effect on later lines as on
  hardware. Compositing is priority-then-BG-index over the backdrop; index 0 of
  a (sub-)palette is transparent.
- **Display timing** is exact at line granularity (1232 cycles/line, 228 lines,
  VBlank at 160). VBlank/HBlank/VCount interrupts fire through the real
  interrupt controller, which is what activates the Phase-2 `IntrWait` SWIs.
  Sub-line dot timing is modeled only enough to place the HBlank flag; the
  precise HBlank IRQ *offset* within a line is refined when the Phase-6
  scheduler needs it.

## Accuracy notes (Phase 4 trade-offs)

- **Sprites**: regular (text) OBJs only — position with H/V wrap, all shapes and
  sizes (8×8 to 64×64), 4bpp/8bpp, per-sprite H/V flip, 1D and 2D tile mapping,
  and priority compositing against the backgrounds (an OBJ wins ties with a
  same-priority BG; a lower OAM index wins sprite-vs-sprite overlap).
- **Deferred to Phase 5** (they share the affine machinery with affine BGs):
  rotation/scaling (affine) sprites, the OBJ window, and OBJ semi-transparent
  blending. Affine sprites are skipped rather than drawn untransformed, so they
  are simply absent until Phase 5; a semi-transparent OBJ renders opaque for now.
- **VRAM 8-bit-write boundary** is now mode-aware: byte writes to OBJ tile VRAM
  are correctly ignored (they only duplicate in the BG area), with the BG/OBJ
  split at 0x10000 in tiled modes and 0x14000 in the bitmap modes.

## Accuracy notes (Phase 6 trade-offs)

- **DMA**: all four channels with immediate, VBlank and HBlank start timing,
  increment/decrement/fixed/reload address modes, 16/32-bit units, repeat, and
  the completion interrupt. Channels are serviced in priority order and each
  transfer runs atomically between CPU instructions — accurate to the CPU's
  view of memory, though DMA does not yet *steal* CPU cycles (deferred to a
  cycle-accurate pass). Sound-FIFO / video-capture "special" timing is Phase 7.
- **Timers**: all four with the four prescalers, cascade (count-up) mode, and
  the overflow interrupt. Counters advance per `tick` (one CPU instruction) with
  the prescaler remainder carried between ticks, so timing is accurate to a
  few-cycle granularity; the exact sub-instruction phase of a timer IRQ is
  refined only if needed.
- **Interrupts** from the PPU, timers and DMA now all flow through the real
  `IE`/`IF`/`IME` controller into the CPU — the machinery a real ROM's main
  loop and `IntrWait` calls depend on.
