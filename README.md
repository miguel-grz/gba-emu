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
| 3–5 | PPU (tiled, sprites, affine/bitmap modes) | — |
| 6 | DMA, timers, interrupt scheduling | — |
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
│   ├── memory.rs       # Bus trait + full memory map, timing, open bus
│   ├── io.rs           # I/O register file (interrupt controller, WAITCNT, …)
│   ├── timing.rs       # waitstate / S-N cycle model
│   ├── bios.rs         # BIOS SWI handling (HLE routines + LLE fallback)
│   └── lib.rs
└── tests/
    ├── cpu_test.rs     # hand-assembled instruction tests + headless ROM harness
    └── bus_test.rs     # I/O, timing, BIOS HLE, open bus, interrupts/halt
```

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
- **Still stubbed**: cartridge SRAM/Flash/EEPROM saves read as 0, and BIOS
  read-protection is not enforced. Neither affects CPU test ROMs.
