# Pocket — a Game Boy Advance emulator

**Pocket** is an open-source Game Boy Advance emulator built around a
cycle-aware ARM7TDMI core written in Rust, compiled to WebAssembly, and wrapped
in a modern, deliberately-designed interface. It runs commercial games
(Pokémon FireRed, Ruby, …) and public-domain homebrew alike, right in the
browser today and as a native desktop app soon.

> Most GBA emulators are excellent at accuracy and stuck in 2008 at
> presentation. Pocket's goal is to keep the accuracy and give the whole thing
> a UI that feels like it was made this decade.

## Why another GBA emulator?

The emulation scene is mature — mGBA and others are superb cores. What's missing
is a player that treats your library like a modern app treats your content:

- **A real library, not a file picker.** Your games live in a grid with cover
  art and clean titles, stored locally in your browser, searchable and
  organizable — not re-opened from disk every time.
- **Automatic cover art & clean names.** Drop in `firered.gba` and it becomes
  *Pokémon FireRed* with its box art. Covers are fetched at runtime from the
  community [libretro thumbnail server](https://thumbnails.libretro.com) (the
  same source RetroArch uses) — nothing copyrighted is bundled, and the source
  is credited in-app. Anything it can't identify falls back to a real
  screen capture of the game.
- **Design as a feature.** A cohesive dark theme, save states, favorites, and
  recently-played — the surface polish that dedicated emulators rarely invest
  in.
- **No BIOS hunting.** The BIOS is emulated at a high level, so most games boot
  with nothing but the ROM.
- **Genuinely open source.** MIT / Apache-2.0, hackable end to end, from the ARM
  decoder to the CSS.

## What works today

- **Full CPU** — ARM7TDMI: complete ARM + Thumb instruction sets, banked
  registers, pipeline, exceptions.
- **Full graphics** — every PPU mode (tiled 0–2, bitmap 3–5), sprites (regular +
  affine), windows, mosaic, and alpha/brighten/darken blending.
- **Sound** — the four PSG channels plus Direct Sound (the PCM path most
  commercial games use for music), mixed and resampled to your browser's audio.
- **DMA, timers, interrupts** — the full machinery real game loops depend on.
- **Cartridge saves** — SRAM and Flash (64K/128K), auto-detected and persisted
  per cartridge, so in-game saves survive reloads.
- **Save states** — snapshot and restore anywhere.
- **Library UI** — IndexedDB-backed game storage, cover art, clean titles,
  inline rename, favorites, recently-played, drag-and-drop import, search.

## Using Pocket

### In the browser (today)

```sh
npm install
npm run wasm     # build the Rust core → WebAssembly (needs wasm-pack)
npm run dev      # start the dev server, then open the printed localhost URL
```

Then just **drag a `.gba` file onto the window** (or use *Add ROM*). Your games
and saves stay in your browser — nothing is uploaded anywhere.

**Default controls**

| Button | Key |
|--------|-----|
| D-Pad | Arrow keys |
| A / B | X / Z |
| L / R | A / S |
| Start / Select | Enter / Backspace |

*(Remappable controls and gamepad support are on the roadmap.)*

### As a desktop app (coming soon)

A [Tauri](https://tauri.app) shell will package Pocket as a small native app for
macOS, Windows, and Linux. Once it lands, this section will cover downloading a
release and opening ROMs directly — no terminal required.

> **Bring your own ROMs.** Pocket ships no games. Use homebrew, or dumps of
> cartridges you own.

## Stack

| Layer | Technology |
|-------|-----------|
| Emulator core | **Rust** (`core/`, crate `gba-core`) — zero UI dependencies, 121 tests |
| Core → web | **WebAssembly** via `wasm-bindgen` / `wasm-pack` (`web/`) |
| Frontend | **React + TypeScript**, built with **Vite** |
| Storage | **IndexedDB** (ROMs + library), `localStorage` (save states + battery) |
| Audio | **Web Audio API** (resampled from the core's 32768 Hz stream) |
| Desktop *(planned)* | **Tauri** |
| Hardware reference | [GBATEK](https://problemkaputt.de/gbatek.htm) |

## Project layout

```
core/            # gba-core — the emulator, pure Rust, no UI
├── src/
│   ├── cpu/     # ARM7TDMI: register file/banking, ARM + Thumb decoders
│   ├── memory.rs, io.rs        # bus, full memory map, I/O registers, timing
│   ├── ppu.rs                  # backgrounds, sprites, windows, blending
│   ├── apu.rs, dma.rs, timers.rs
│   ├── bios.rs, save.rs        # BIOS HLE (incl. decompression SWIs), SRAM/Flash
│   └── system.rs               # ties it together, run_frame loop
└── tests/       # 121 self-contained tests + headless ROM harness

web/             # WebAssembly bindings (wasm-bindgen)
src/             # React + TypeScript frontend
├── components/  # Sidebar, Library, GameCard, Console, Settings, …
└── lib/         # gba.ts (runner), library.ts (IndexedDB), gamedb.ts (titles/art)
```

## Roadmap

| Status | Item |
|:------:|------|
| ✅ | ARM7TDMI CPU (ARM + Thumb, banking, pipeline, exceptions) |
| ✅ | Memory/bus, I/O, waitstate timing, BIOS HLE + LLE |
| ✅ | PPU — tiled + bitmap modes, sprites, affine, windows, mosaic, blending |
| ✅ | DMA, timers, interrupts |
| ✅ | APU — PSG channels + Direct Sound |
| ✅ | Web frontend — library, covers, save states, saves, redesigned UI |
| 🔜 | Configurable controls + gamepad support |
| 🔜 | Tauri desktop shell |
| 💡 | EEPROM saves, cartridge prefetch, cycle-accurate DMA stalls |

**Pocket is under active development** — expect continuous improvements to
accuracy, performance, and the interface. Issues and pull requests are welcome.

## Building & testing the core

The Rust core is fully testable on its own, no browser or ROM required:

```sh
cargo test                                   # 121 tests
cargo run --example render_scene -- scene.bmp   # render a tiled scene to a BMP
cargo run --example render_rom -- rom.gba out.bmp   # dump a real ROM's first frame
```

The suite covers ALU flags and barrel-shifter edge cases, pipeline-visible PC
offsets, LDM/STM quirks, mode banking and interworking, the I/O registers and
waitstate timing, BIOS HLE routines, PPU rendering and display IRQs, DMA and
timers, and the audio sample stream.

### Optional test ROMs

ROMs are not redistributed here. Drop these into `core/tests/roms/` and the
harness picks them up automatically (tests skip with a notice otherwise):

| File | Source | What it gives us |
|------|--------|------------------|
| `arm.gba`, `thumb.gba` | [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) | Real pass/fail assertions — on failure the ROM parks with the failing test number in `r12` |
| `hello.gba` | jsmolka | Boots and renders "Hello world!" in mode 4 — a quick end-to-end smoke test |

No BIOS image is required; the core boots ROMs from the post-BIOS state and
emulates BIOS SWIs at a high level. Load a real BIOS via `Memory::load_bios` for
low-level BIOS execution.

## License

Dual-licensed under **MIT** or **Apache-2.0**, at your option.

## Credits & references

- [GBATEK](https://problemkaputt.de/gbatek.htm) — the definitive GBA hardware
  reference.
- [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) — CPU test ROMs.
- [libretro thumbnails](https://thumbnails.libretro.com) — community cover-art
  server used for library art at runtime.

---

## Appendix — accuracy notes & trade-offs

Deeper notes on what each subsystem models exactly and what it approximates, for
the curious (and for future-me).

### CPU

- **Pipeline**: modeled as a two-slot fetch queue — every architecturally
  visible effect is correct (PC reads +8/+4, stores of PC +12, flush on branch,
  stale opcodes for self-modifying code).
- **ARM7 quirks implemented**: misaligned LDR/LDRH rotation, LDRSH→LDRSB at odd
  addresses, LDM/STM empty-list and base-in-list behavior, user-bank transfers
  (S bit), `MOV pc`/`POP {pc}` non-interworking on ARMv4T.
- **Deliberately unpredictable-as-benign**: MSR cannot flip the T bit, invalid
  mode writes keep the old mode, ARMv5-only encodings (BLX, LDRD) take the
  undefined trap or act as no-ops per hardware.

### Memory / bus

- **Cycle counts** are driven by the bus: memory-access cycles come from the
  [`timing`](core/src/timing.rs) waitstate model; instruction handlers add only
  internal (I-)cycles. The cartridge **prefetch buffer** is not modeled yet, so
  ROM-heavy code runs a few percent *slow* (a conservative error).
- **BIOS**: HLE by default (no copyrighted image needed). `Div`/`Sqrt`/`CpuSet`/
  `CpuFastSet`/`RegisterRamReset` are exact; `ArcTan`/`ArcTan2` use the
  mathematically correct result; `IntrWait`/`VBlankIntrWait` halt until an
  interrupt; LZ77/RLE/Huffman/diff-unfilter/BitUnpack decompression SWIs are
  implemented (needed by real games).
- **8-bit-write quirks**: palette and BG-VRAM byte writes duplicate across the
  halfword; OAM ignores byte writes; the BG/OBJ VRAM boundary is mode-dependent.

### PPU

- Covers the full tiled/bitmap feature set: bitmap modes 3–5; affine BG2/BG3
  with per-scanline reference-point updates and wrap/transparent overflow;
  affine sprites including double-size; BG and OBJ mosaic; windows (WIN0/WIN1/OBJ
  window); and the color special effects — alpha blending, brighten, darken, and
  OBJ semi-transparency.
- **Compositing** renders each layer into its own scanline buffer, then per
  pixel picks the front-most and second layers (after window masking) and
  applies the blend — the approach that makes two-target blending clean, at the
  cost of five full-width buffers per line.
- **Display timing** is exact at line granularity (1232 cycles/line, 228 lines,
  VBlank at 160); VBlank/HBlank/VCount IRQs fire through the real interrupt
  controller. Sub-line dot timing is modeled only enough to place the HBlank flag.
- Affine-sprite mosaic is applied in texture space (a close approximation of
  hardware's screen-space mosaic).

### APU

- The four PSG channels — two square (channel 1 with frequency sweep), the wave
  channel, and the noise LFSR — each with volume envelope and length counter,
  clocked by a 512 Hz frame sequencer.
- **Direct Sound** A/B: two 8-bit PCM FIFOs clocked by a timer overflow
  (SOUNDCNT_H selects the timer), refilled by DMA1/DMA2 on the "special"
  start-timing when half-empty — what most commercial games use for music.
- Everything mixes to stereo and resamples to 32768 Hz. Mixing levels are a
  reasonable approximation, not calibrated against hardware's exact DAC/SOUNDBIAS
  response.

### DMA / timers / interrupts

- **DMA**: all four channels with immediate/VBlank/HBlank/special start timing,
  every address mode, 16/32-bit units, repeat, and completion IRQ. Transfers run
  atomically between CPU instructions; DMA does not yet *steal* CPU cycles
  (deferred to a cycle-accurate pass).
- **Timers**: all four with the four prescalers, cascade (count-up) mode, and the
  overflow interrupt, accurate to a few-cycle granularity.
- **Interrupts** from the PPU, timers, and DMA all flow through the real
  `IE`/`IF`/`IME` controller into the CPU.

### Saves

- **SRAM** (32K) and **Flash** (64K Panasonic / 128K Sanyo, with the full command
  protocol, chip ID, sector erase, and bank switching) are auto-detected from the
  ROM's save-type marker and persisted per cartridge. **EEPROM** is not yet
  implemented.
