# Accuracy notes & trade-offs

Deeper notes on what each subsystem models exactly and what it approximates —
for the curious, and for future-me. For the high-level overview, see the
[README](../README.md).

## CPU

- **Pipeline**: modeled as a two-slot fetch queue — every architecturally
  visible effect is correct (PC reads +8/+4, stores of PC +12, flush on branch,
  stale opcodes for self-modifying code).
- **ARM7 quirks implemented**: misaligned LDR/LDRH rotation, LDRSH→LDRSB at odd
  addresses, LDM/STM empty-list and base-in-list behavior, user-bank transfers
  (S bit), `MOV pc`/`POP {pc}` non-interworking on ARMv4T.
- **Deliberately unpredictable-as-benign**: MSR cannot flip the T bit, invalid
  mode writes keep the old mode, ARMv5-only encodings (BLX, LDRD) take the
  undefined trap or act as no-ops per hardware.

## Memory / bus

- **Cycle counts** are driven by the bus: memory-access cycles come from the
  [`timing`](../core/src/timing.rs) waitstate model; instruction handlers add
  only internal (I-)cycles. The cartridge **prefetch buffer** is not modeled yet,
  so ROM-heavy code runs a few percent *slow* (a conservative error).
- **BIOS**: HLE by default (no copyrighted image needed). `Div`/`Sqrt`/`CpuSet`/
  `CpuFastSet`/`RegisterRamReset` are exact; `ArcTan`/`ArcTan2` use the
  mathematically correct result; `IntrWait`/`VBlankIntrWait` halt until an
  interrupt; LZ77/RLE/Huffman/diff-unfilter/BitUnpack decompression SWIs are
  implemented (needed by real games).
- **8-bit-write quirks**: palette and BG-VRAM byte writes duplicate across the
  halfword; OAM ignores byte writes; the BG/OBJ VRAM boundary is mode-dependent.

## PPU

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

## APU

- The four PSG channels — two square (channel 1 with frequency sweep), the wave
  channel, and the noise LFSR — each with volume envelope and length counter,
  clocked by a 512 Hz frame sequencer.
- **Direct Sound** A/B: two 8-bit PCM FIFOs clocked by a timer overflow
  (SOUNDCNT_H selects the timer), refilled by DMA1/DMA2 on the "special"
  start-timing when half-empty — what most commercial games use for music.
- Everything mixes to stereo and resamples to 32768 Hz. Mixing levels are a
  reasonable approximation, not calibrated against hardware's exact DAC/SOUNDBIAS
  response.

## DMA / timers / interrupts

- **DMA**: all four channels with immediate/VBlank/HBlank/special start timing,
  every address mode, 16/32-bit units, repeat, and completion IRQ. Transfers run
  atomically between CPU instructions; DMA does not yet *steal* CPU cycles
  (deferred to a cycle-accurate pass).
- **Timers**: all four with the four prescalers, cascade (count-up) mode, and the
  overflow interrupt, accurate to a few-cycle granularity.
- **Interrupts** from the PPU, timers, and DMA all flow through the real
  `IE`/`IF`/`IME` controller into the CPU.

## Saves

- **SRAM** (32K) and **Flash** (64K Panasonic / 128K Sanyo, with the full command
  protocol, chip ID, sector erase, and bank switching) are auto-detected from the
  ROM's save-type marker and persisted per cartridge. **EEPROM** is not yet
  implemented.
