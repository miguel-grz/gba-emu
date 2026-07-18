//! Waitstate / access-timing model.
//!
//! Every memory access costs a number of CPU cycles that depends on the
//! region, the access width, whether it is sequential (S) or non-sequential
//! (N), and — for the cartridge and SRAM — the software-programmed `WAITCNT`.
//! This module turns those inputs into a cycle count; [`crate::memory::Memory`]
//! feeds it and accumulates the result so the CPU's cycle counter reflects
//! real bus timing instead of the flat per-access estimate used in Phase 1.
//!
//! ## Approximation, stated honestly
//!
//! Sequential vs non-sequential is inferred from address adjacency (an access
//! to the word immediately after the previous one, in the same region, is
//! treated as sequential). This reproduces the common case — linear code
//! fetches and array walks are S, branch targets and random accesses are N —
//! without threading an explicit S/N flag out of the CPU pipeline.
//!
//! What is deliberately *not* modeled yet: the cartridge **prefetch buffer**
//! (WAITCNT bit 14), which on hardware lets sequential ROM fetches overlap and
//! hides waitstates. Ignoring it makes ROM-heavy code run a few percent slow
//! rather than fast — a conservative error, and the honest place to fix it is
//! alongside the DMA/timer scheduler in Phase 6, where prefetch stalls
//! actually interact with other bus masters. The N/S structure is in place so
//! that addition does not require touching the CPU.

/// Access width in bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

/// A coarse region tag used only for timing (distinct from address decode).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 32-bit bus, no waitstates: BIOS, IWRAM, I/O, OAM.
    Fast,
    /// 16-bit bus, 2 waitstates: on-board EWRAM.
    Ewram,
    /// 16-bit bus, no waitstates: palette RAM and VRAM.
    Video,
    /// Cartridge ROM waitstate set 0/1/2 (mirror of the same ROM).
    Rom0,
    Rom1,
    Rom2,
    /// Cartridge SRAM (8-bit bus).
    Sram,
}

impl Region {
    /// Map a 32-bit address to its timing region.
    pub fn of(addr: u32) -> Region {
        match addr >> 24 {
            0x00 | 0x03 | 0x04 | 0x07 => Region::Fast,
            0x02 => Region::Ewram,
            0x05 | 0x06 => Region::Video,
            0x08 | 0x09 => Region::Rom0,
            0x0A | 0x0B => Region::Rom1,
            0x0C | 0x0D => Region::Rom2,
            0x0E | 0x0F => Region::Sram,
            _ => Region::Fast,
        }
    }
}

// WAITCNT lookup tables (GBATEK). Values are the *wait* added to a 1-cycle
// base, so the first-access cost is `1 + N`.
const N_WAITS: [u32; 4] = [4, 3, 2, 8];
const WS0_S: [u32; 2] = [2, 1];
const WS1_S: [u32; 2] = [4, 1];
const WS2_S: [u32; 2] = [8, 1];
const SRAM_WAITS: [u32; 4] = [4, 3, 2, 8];

/// Cycles for a single access. `waitcnt` is the raw 16-bit register value.
pub fn access_cycles(region: Region, width: Width, seq: bool, waitcnt: u16) -> u32 {
    match region {
        Region::Fast => 1,
        Region::Video => match width {
            Width::Word => 2, // 16-bit bus: two halfword accesses
            _ => 1,
        },
        Region::Ewram => match width {
            Width::Word => 6, // 2 waitstates on a 16-bit bus, doubled for 32-bit
            _ => 3,
        },
        Region::Sram => {
            // 8-bit bus; only byte access is meaningful, but cost the wait
            // regardless of width so mis-sized accesses are not free.
            1 + SRAM_WAITS[(waitcnt & 0x3) as usize]
        }
        Region::Rom0 | Region::Rom1 | Region::Rom2 => {
            let (n_bits, s_table, s_bit) = match region {
                Region::Rom0 => ((waitcnt >> 2) & 0x3, &WS0_S, (waitcnt >> 4) & 1),
                Region::Rom1 => ((waitcnt >> 5) & 0x3, &WS1_S, (waitcnt >> 7) & 1),
                _ => ((waitcnt >> 8) & 0x3, &WS2_S, (waitcnt >> 10) & 1),
            };
            let n = 1 + N_WAITS[n_bits as usize];
            let s = 1 + s_table[s_bit as usize];
            match width {
                // A 32-bit ROM access is two 16-bit accesses: the first at the
                // access's own S/N cost, the second always sequential.
                Width::Word => (if seq { s } else { n }) + s,
                _ => {
                    if seq {
                        s
                    } else {
                        n
                    }
                }
            }
        }
    }
}

/// Tracks the previous access so the next one can be classified S or N.
#[derive(Default)]
pub struct SeqTracker {
    /// Address where the previous access ended, and the region it was in.
    prev_end: Option<(u32, u8)>,
}

impl SeqTracker {
    /// Classify `addr`/`width` and update state. Sequential iff this access
    /// begins exactly where the previous one ended, within the same region.
    pub fn classify(&mut self, addr: u32, width: Width) -> bool {
        let size = match width {
            Width::Byte => 1,
            Width::Half => 2,
            Width::Word => 4,
        };
        let region = addr >> 24;
        let seq = self.prev_end == Some((addr, region as u8));
        self.prev_end = Some((addr.wrapping_add(size), region as u8));
        seq
    }
}
