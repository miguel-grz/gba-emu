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
| 2 | Memory/bus (waitstates, open bus, BIOS HLE/LLE) | minimal stub |
| 3–5 | PPU (tiled, sprites, affine/bitmap modes) | — |
| 6 | DMA, timers, interrupt wiring | — |
| 7 | APU | — |
| 8 | Tauri shell + UI | — |

## Layout

```
core/
├── src/
│   ├── cpu/
│   │   ├── mod.rs      # register file, banking, CPSR/SPSR, pipeline, shifter
│   │   ├── arm.rs      # 32-bit ARM instruction set
│   │   └── thumb.rs    # 16-bit Thumb instruction set
│   ├── memory.rs       # Bus trait + minimal Phase-1 memory map
│   └── lib.rs
└── tests/
    └── cpu_test.rs     # hand-assembled instruction tests + headless ROM harness
```

## Building and testing

```sh
cargo test
```

The suite is self-contained: 30+ hand-assembled instruction tests cover ALU
flags, barrel-shifter edge cases, pipeline-visible PC offsets, misaligned
load rotation, LDM/STM quirks, mode banking, exceptions and ARM↔Thumb
interworking.

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
  branch, stale opcodes for self-modifying code), but per-stage bus timing
  is deferred to the Phase 2 bus work.
- **Cycle counts**: `Cpu::step` returns approximate cycles (S/N/I structure
  without waitstates). Accurate timing needs the real bus; the counter exists
  now so the PPU/timers can schedule against it later.
- **ARM7 quirks implemented**: misaligned LDR/LDRH rotation, LDRSH→LDRSB at
  odd addresses, LDM/STM empty-list and base-in-list behavior, user-bank
  transfers (S bit), `MOV pc`/`POP {pc}` non-interworking on ARMv4T.
- **Deliberately unpredictable-as-benign**: MSR cannot flip the T bit,
  invalid mode writes keep the old mode, ARMv5-only encodings (BLX, LDRD)
  take the undefined trap or act as no-ops per hardware.
