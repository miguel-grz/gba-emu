//! ARM7TDMI CPU core.
//!
//! This module owns the register file (including per-mode banked registers),
//! CPSR/SPSR handling, exception entry, pipeline bookkeeping and the barrel
//! shifter. Instruction decode/execute lives in [`arm`] (32-bit set) and
//! [`thumb`] (16-bit set).
//!
//! ## Pipeline model
//!
//! The real ARM7TDMI has a 3-stage fetch/decode/execute pipeline. We model it
//! as a two-slot fetch queue: `pipeline[0]` is the instruction about to
//! execute, `pipeline[1]` the one being decoded, and `regs[15]` points at the
//! current fetch address. This reproduces every architecturally visible
//! effect — PC reads as instruction address + 8 (ARM) / + 4 (Thumb), stores of
//! PC see + 12, branches flush and refill, and self-modifying code sees stale
//! prefetched opcodes — without simulating per-stage bus activity. What it
//! does *not* reproduce is exact prefetch *timing* against waitstates; that is
//! deferred to Phase 2 alongside the real bus, where it belongs.
//!
//! ## Cycle counting
//!
//! `step` returns an approximate cycle count (1S per instruction, plus extra
//! cycles for loads/stores, multiplies, register-specified shifts and pipeline
//! refills, all treated as 1-cycle accesses). Accurate S/N/I timing requires
//! waitstate-aware memory, so the honest per-region numbers arrive with the
//! Phase 2 bus. The counter exists now so the PPU/timers can be scheduled
//! against it later without reworking the CPU.

pub mod arm;
pub mod thumb;

use crate::memory::Bus;

pub(crate) const FLAG_N: u32 = 1 << 31;
pub(crate) const FLAG_Z: u32 = 1 << 30;
pub(crate) const FLAG_C: u32 = 1 << 29;
pub(crate) const FLAG_V: u32 = 1 << 28;
pub(crate) const FLAG_I: u32 = 1 << 7;
pub(crate) const FLAG_F: u32 = 1 << 6;
pub(crate) const FLAG_T: u32 = 1 << 5;
const MODE_MASK: u32 = 0x1F;

pub(crate) const VEC_RESET: u32 = 0x00;
pub(crate) const VEC_UNDEFINED: u32 = 0x04;
pub(crate) const VEC_SWI: u32 = 0x08;
pub(crate) const VEC_IRQ: u32 = 0x18;
pub(crate) const VEC_FIQ: u32 = 0x1C;

/// ARM7TDMI processor modes (CPSR bits 4:0).
///
/// System mode is included even though it is often omitted from summaries:
/// it shares the User register bank but is privileged, and the GBA BIOS
/// hands control to games in System mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    User,
    Fiq,
    Irq,
    Supervisor,
    Abort,
    Undefined,
    System,
}

impl Mode {
    pub fn bits(self) -> u32 {
        match self {
            Mode::User => 0x10,
            Mode::Fiq => 0x11,
            Mode::Irq => 0x12,
            Mode::Supervisor => 0x13,
            Mode::Abort => 0x17,
            Mode::Undefined => 0x1B,
            Mode::System => 0x1F,
        }
    }

    pub fn from_bits(bits: u32) -> Option<Mode> {
        match bits & MODE_MASK {
            0x10 => Some(Mode::User),
            0x11 => Some(Mode::Fiq),
            0x12 => Some(Mode::Irq),
            0x13 => Some(Mode::Supervisor),
            0x17 => Some(Mode::Abort),
            0x1B => Some(Mode::Undefined),
            0x1F => Some(Mode::System),
            _ => None,
        }
    }

    fn spsr_index(self) -> Option<usize> {
        match self {
            Mode::Fiq => Some(0),
            Mode::Irq => Some(1),
            Mode::Supervisor => Some(2),
            Mode::Abort => Some(3),
            Mode::Undefined => Some(4),
            Mode::User | Mode::System => None,
        }
    }

    fn is_privileged(self) -> bool {
        !matches!(self, Mode::User)
    }
}

/// ARM7TDMI CPU state.
pub struct Cpu {
    /// Active register view for the current mode. `regs[15]` is the fetch
    /// address (see the pipeline notes in the module docs).
    regs: [u32; 16],
    cpsr: u32,
    /// SPSRs for FIQ, IRQ, Supervisor, Abort, Undefined (see `spsr_index`).
    spsr: [u32; 5],
    /// User/System bank: r8–r14 (r8–r12 shared with all non-FIQ modes).
    bank_usr: [u32; 7],
    /// FIQ bank: r8–r14.
    bank_fiq: [u32; 7],
    bank_irq: [u32; 2],
    bank_svc: [u32; 2],
    bank_abt: [u32; 2],
    bank_und: [u32; 2],
    /// Two-slot fetch queue: `[0]` executes next, `[1]` is in decode.
    pipeline: [u32; 2],
    /// Set when the executing instruction wrote PC (pipeline was flushed).
    branched: bool,
    /// CPU is halted (Halt SWI / HALTCNT), waiting for an interrupt.
    halted: bool,
    cycles: u64,
}

impl Cpu {
    /// Create a CPU in the post-reset state. The pipeline is empty; call
    /// [`Cpu::reset`], [`Cpu::skip_bios`] or [`Cpu::jump`] with a bus before
    /// stepping.
    pub fn new() -> Self {
        Cpu {
            regs: [0; 16],
            cpsr: Mode::Supervisor.bits() | FLAG_I | FLAG_F,
            spsr: [0; 5],
            bank_usr: [0; 7],
            bank_fiq: [0; 7],
            bank_irq: [0; 2],
            bank_svc: [0; 2],
            bank_abt: [0; 2],
            bank_und: [0; 2],
            pipeline: [0; 2],
            branched: false,
            halted: false,
            cycles: 0,
        }
    }

    /// Hardware reset: Supervisor mode, IRQ/FIQ disabled, ARM state, PC at the
    /// reset vector.
    pub fn reset<B: Bus>(&mut self, bus: &mut B) {
        self.set_cpsr(Mode::Supervisor.bits() | FLAG_I | FLAG_F);
        self.branch_to(bus, VEC_RESET);
    }

    /// Enter the state the BIOS leaves the CPU in when it jumps to a
    /// cartridge: System mode, IRQs enabled, stacks set up, PC at ROM start.
    /// Lets ROMs run without a real BIOS image.
    pub fn skip_bios<B: Bus>(&mut self, bus: &mut B) {
        self.set_cpsr(Mode::System.bits());
        self.regs[13] = 0x0300_7F00;
        self.regs[14] = 0x0800_0000;
        self.bank_irq = [0x0300_7FA0, 0];
        self.bank_svc = [0x0300_7FE0, 0];
        self.branch_to(bus, 0x0800_0000);
    }

    /// Execute one instruction (or service an interrupt, or idle while
    /// halted); returns the cycles it took. Memory-access cycles come from the
    /// bus's waitstate model; internal (I-)cycles are added by the instruction
    /// handlers.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> u64 {
        let start = self.cycles;

        // An unmasked pending interrupt is serviced before the next fetch, and
        // wakes the CPU from halt.
        if bus.irq_pending() && self.cpsr & FLAG_I == 0 {
            self.halted = false;
            self.irq(bus);
            self.cycles += u64::from(bus.take_access_cycles());
            return self.cycles - start;
        }

        // Halted: nothing runs until an interrupt arrives. Account one cycle of
        // idle time so callers scheduling against `cycles()` still advance.
        if self.halted {
            self.cycles += 1;
            return self.cycles - start;
        }

        self.branched = false;
        if self.is_thumb() {
            let op = self.pipeline[0] as u16;
            self.pipeline[0] = self.pipeline[1];
            self.pipeline[1] = u32::from(bus.read16(self.regs[15] & !1));
            thumb::execute(self, bus, op);
            if !self.branched {
                self.regs[15] = self.regs[15].wrapping_add(2);
            }
        } else {
            let op = self.pipeline[0];
            self.pipeline[0] = self.pipeline[1];
            self.pipeline[1] = bus.read32(self.regs[15] & !3);
            if self.check_cond(op >> 28) {
                arm::execute(self, bus, op);
            }
            if !self.branched {
                self.regs[15] = self.regs[15].wrapping_add(4);
            }
        }
        // Drain the waitstate cycles from this instruction's fetch, pipeline
        // refill and data accesses.
        self.cycles += u64::from(bus.take_access_cycles());
        // A HALTCNT write during this instruction (LLE path) halts the CPU.
        if bus.take_halt_request() {
            self.halted = true;
        }
        self.cycles - start
    }

    /// Whether the CPU is currently halted (waiting for an interrupt).
    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// Request halt (used by the HLE Halt/IntrWait SWIs).
    pub(crate) fn request_halt(&mut self) {
        self.halted = true;
    }

    /// Dispatch a `SWI`: run the real BIOS (LLE) if one is loaded, otherwise
    /// emulate the routine in software (HLE).
    pub(crate) fn do_swi<B: Bus>(&mut self, bus: &mut B, number: u8) {
        if bus.has_bios() {
            let lr = if self.is_thumb() {
                self.regs[15].wrapping_sub(2)
            } else {
                self.regs[15].wrapping_sub(4)
            };
            self.enter_exception(bus, VEC_SWI, Mode::Supervisor, lr);
        } else {
            crate::bios::hle_swi(self, bus, number);
        }
    }

    /// Reset into System mode / ARM state and branch to `target` (used by the
    /// HLE SoftReset SWI).
    pub(crate) fn reset_to<B: Bus>(&mut self, bus: &mut B, target: u32) {
        self.halted = false;
        self.set_cpsr(Mode::System.bits());
        self.branch_to(bus, target);
    }

    /// Signal a normal interrupt. Taken unless the I flag masks it.
    pub fn irq<B: Bus>(&mut self, bus: &mut B) {
        if self.cpsr & FLAG_I != 0 {
            return;
        }
        // Return address such that the handler's `SUBS PC, LR, #4` resumes
        // the not-yet-executed instruction, in both states.
        let lr = if self.is_thumb() {
            self.regs[15]
        } else {
            self.regs[15].wrapping_sub(4)
        };
        self.enter_exception(bus, VEC_IRQ, Mode::Irq, lr);
    }

    /// Signal a fast interrupt. Taken unless the F flag masks it.
    /// (Nothing on a stock GBA raises FIQ, but the core supports it.)
    pub fn fiq<B: Bus>(&mut self, bus: &mut B) {
        if self.cpsr & FLAG_F != 0 {
            return;
        }
        let lr = if self.is_thumb() {
            self.regs[15]
        } else {
            self.regs[15].wrapping_sub(4)
        };
        self.enter_exception(bus, VEC_FIQ, Mode::Fiq, lr);
    }

    /// General-purpose register read (current mode's view). `reg(15)` returns
    /// the raw pipeline-relative value; use [`Cpu::pc`] for the address of the
    /// next instruction to execute.
    pub fn reg(&self, index: usize) -> u32 {
        self.regs[index & 0xF]
    }

    pub fn set_reg(&mut self, index: usize, value: u32) {
        self.regs[index & 0xF] = value;
    }

    /// Address of the next instruction that will execute.
    pub fn pc(&self) -> u32 {
        self.regs[15].wrapping_sub(if self.is_thumb() { 4 } else { 8 })
    }

    /// Branch to `target` and refill the pipeline (respects the current
    /// ARM/Thumb state).
    pub fn jump<B: Bus>(&mut self, bus: &mut B, target: u32) {
        self.branch_to(bus, target);
    }

    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    /// Full CPSR write, including mode changes (registers are re-banked) and
    /// the T bit. Instruction-level restrictions (e.g. MSR ignoring T, User
    /// mode only touching flags) are enforced by the instruction handlers,
    /// not here.
    pub fn set_cpsr(&mut self, value: u32) {
        let old_mode = self.mode();
        // An invalid mode pattern is architecturally unpredictable; we keep
        // the old mode bits rather than corrupting bank state.
        let value = match Mode::from_bits(value) {
            Some(new_mode) => {
                self.switch_bank(old_mode, new_mode);
                value
            }
            None => (value & !MODE_MASK) | (self.cpsr & MODE_MASK),
        };
        self.cpsr = value;
    }

    /// Current mode's SPSR. In User/System (which have none) this returns the
    /// CPSR, mirroring the common ARM7 reading of the unpredictable case.
    pub fn spsr(&self) -> u32 {
        match self.mode().spsr_index() {
            Some(i) => self.spsr[i],
            None => self.cpsr,
        }
    }

    pub(crate) fn set_spsr(&mut self, value: u32) {
        if let Some(i) = self.mode().spsr_index() {
            self.spsr[i] = value;
        }
    }

    pub fn mode(&self) -> Mode {
        // Invariant: set_cpsr never stores an invalid mode pattern.
        Mode::from_bits(self.cpsr).unwrap_or(Mode::System)
    }

    pub fn is_thumb(&self) -> bool {
        self.cpsr & FLAG_T != 0
    }

    pub fn flag_n(&self) -> bool {
        self.cpsr & FLAG_N != 0
    }
    pub fn flag_z(&self) -> bool {
        self.cpsr & FLAG_Z != 0
    }
    pub fn flag_c(&self) -> bool {
        self.cpsr & FLAG_C != 0
    }
    pub fn flag_v(&self) -> bool {
        self.cpsr & FLAG_V != 0
    }

    /// Total cycles executed since power-on (approximate; see module docs).
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    // ---- internals shared by arm.rs / thumb.rs ----

    pub(crate) fn set_flag(&mut self, flag: u32, on: bool) {
        if on {
            self.cpsr |= flag;
        } else {
            self.cpsr &= !flag;
        }
    }

    pub(crate) fn set_nz(&mut self, value: u32) {
        self.set_flag(FLAG_N, value & (1 << 31) != 0);
        self.set_flag(FLAG_Z, value == 0);
    }

    pub(crate) fn reg_mut(&mut self, index: usize) -> &mut u32 {
        &mut self.regs[index & 0xF]
    }

    pub(crate) fn add_cycles(&mut self, n: u64) {
        self.cycles += n;
    }

    pub(crate) fn check_cond(&self, cond: u32) -> bool {
        let n = self.flag_n();
        let z = self.flag_z();
        let c = self.flag_c();
        let v = self.flag_v();
        match cond & 0xF {
            0x0 => z,
            0x1 => !z,
            0x2 => c,
            0x3 => !c,
            0x4 => n,
            0x5 => !n,
            0x6 => v,
            0x7 => !v,
            0x8 => c && !z,
            0x9 => !c || z,
            0xA => n == v,
            0xB => n != v,
            0xC => !z && n == v,
            0xD => z || n != v,
            0xE => true,
            // 0xF is reserved on ARMv4; treat as never-execute.
            _ => false,
        }
    }

    /// Write PC and refill the pipeline according to the current T bit.
    pub(crate) fn branch_to<B: Bus>(&mut self, bus: &mut B, target: u32) {
        if self.is_thumb() {
            let t = target & !1;
            self.pipeline[0] = u32::from(bus.read16(t));
            self.pipeline[1] = u32::from(bus.read16(t.wrapping_add(2)));
            self.regs[15] = t.wrapping_add(4);
        } else {
            let t = target & !3;
            self.pipeline[0] = bus.read32(t);
            self.pipeline[1] = bus.read32(t.wrapping_add(4));
            self.regs[15] = t.wrapping_add(8);
        }
        self.branched = true;
        // The two refill fetches above are real bus accesses, so their
        // waitstate cost is counted by the memory model, not added here.
    }

    /// Enter an exception: bank-switch, save CPSR into the new mode's SPSR,
    /// set LR, mask IRQs (and FIQs where applicable), force ARM state, and
    /// jump to the vector.
    pub(crate) fn enter_exception<B: Bus>(
        &mut self,
        bus: &mut B,
        vector: u32,
        mode: Mode,
        lr: u32,
    ) {
        let old_cpsr = self.cpsr;
        let mut new_cpsr = (old_cpsr & !(MODE_MASK | FLAG_T)) | mode.bits() | FLAG_I;
        if mode == Mode::Fiq || vector == VEC_RESET {
            new_cpsr |= FLAG_F;
        }
        self.set_cpsr(new_cpsr);
        self.set_spsr(old_cpsr);
        self.regs[14] = lr;
        self.branch_to(bus, vector);
    }

    /// Read a register as User mode sees it (for LDM/STM with the S bit).
    pub(crate) fn reg_user(&self, index: usize) -> u32 {
        match index {
            8..=12 if self.mode() == Mode::Fiq => self.bank_usr[index - 8],
            13 | 14 if !matches!(self.mode(), Mode::User | Mode::System) => {
                self.bank_usr[index - 8]
            }
            _ => self.regs[index],
        }
    }

    pub(crate) fn set_reg_user(&mut self, index: usize, value: u32) {
        match index {
            8..=12 if self.mode() == Mode::Fiq => self.bank_usr[index - 8] = value,
            13 | 14 if !matches!(self.mode(), Mode::User | Mode::System) => {
                self.bank_usr[index - 8] = value
            }
            _ => self.regs[index] = value,
        }
    }

    /// Swap the active r8–r14 view when changing modes. User and System share
    /// a bank; FIQ banks r8–r14, every other exception mode banks r13–r14.
    fn switch_bank(&mut self, from: Mode, to: Mode) {
        if from == to {
            return;
        }
        // Save the outgoing mode's registers.
        if from == Mode::Fiq {
            self.bank_fiq.copy_from_slice(&self.regs[8..15]);
        } else {
            self.bank_usr[..5].copy_from_slice(&self.regs[8..13]);
            let sp_lr = [self.regs[13], self.regs[14]];
            match from {
                Mode::User | Mode::System => {
                    self.bank_usr[5] = sp_lr[0];
                    self.bank_usr[6] = sp_lr[1];
                }
                Mode::Irq => self.bank_irq = sp_lr,
                Mode::Supervisor => self.bank_svc = sp_lr,
                Mode::Abort => self.bank_abt = sp_lr,
                Mode::Undefined => self.bank_und = sp_lr,
                Mode::Fiq => unreachable!(),
            }
        }
        // Load the incoming mode's registers.
        if to == Mode::Fiq {
            self.regs[8..15].copy_from_slice(&self.bank_fiq);
        } else {
            self.regs[8..13].copy_from_slice(&self.bank_usr[..5]);
            let sp_lr = match to {
                Mode::User | Mode::System => [self.bank_usr[5], self.bank_usr[6]],
                Mode::Irq => self.bank_irq,
                Mode::Supervisor => self.bank_svc,
                Mode::Abort => self.bank_abt,
                Mode::Undefined => self.bank_und,
                Mode::Fiq => unreachable!(),
            };
            self.regs[13] = sp_lr[0];
            self.regs[14] = sp_lr[1];
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

// ---- barrel shifter & ALU helpers (shared by ARM and Thumb) ----
//
// Each shift returns `(result, carry_out)`. An amount of zero means "no
// shift, carry unchanged" — callers that need the special ARM immediate
// encodings (LSR #0 = LSR #32, ASR #0 = ASR #32, ROR #0 = RRX) resolve them
// before calling.

pub(crate) fn lsl(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    match amount {
        0 => (value, carry_in),
        1..=31 => (value << amount, value >> (32 - amount) & 1 != 0),
        32 => (0, value & 1 != 0),
        _ => (0, false),
    }
}

pub(crate) fn lsr(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    match amount {
        0 => (value, carry_in),
        1..=31 => (value >> amount, value >> (amount - 1) & 1 != 0),
        32 => (0, value >> 31 != 0),
        _ => (0, false),
    }
}

pub(crate) fn asr(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    match amount {
        0 => (value, carry_in),
        1..=31 => (
            ((value as i32) >> amount) as u32,
            value >> (amount - 1) & 1 != 0,
        ),
        _ => {
            let sign = value >> 31 != 0;
            (if sign { u32::MAX } else { 0 }, sign)
        }
    }
}

pub(crate) fn ror(value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    if amount == 0 {
        (value, carry_in)
    } else if amount.is_multiple_of(32) {
        (value, value >> 31 != 0)
    } else {
        let r = value.rotate_right(amount % 32);
        (r, r >> 31 != 0)
    }
}

/// Rotate-right-extended: 33-bit rotate through carry, by one.
pub(crate) fn rrx(value: u32, carry_in: bool) -> (u32, bool) {
    ((u32::from(carry_in) << 31) | (value >> 1), value & 1 != 0)
}

/// `a + b + carry`, returning `(result, carry_out, overflow)`.
/// Subtraction is `add_with_carry(a, !b, 1)`; subtract-with-borrow passes the
/// C flag as carry, matching the ARM definition exactly.
pub(crate) fn add_with_carry(a: u32, b: u32, carry: u32) -> (u32, bool, bool) {
    let result = a.wrapping_add(b).wrapping_add(carry);
    let carry_out = (u64::from(a) + u64::from(b) + u64::from(carry)) > u64::from(u32::MAX);
    let overflow = (a ^ result) & (b ^ result) & (1 << 31) != 0;
    (result, carry_out, overflow)
}

/// Word load with the ARM7 misalignment behavior: the aligned word rotated so
/// the addressed byte lands in bits 7:0.
pub(crate) fn load32_rotated<B: Bus>(bus: &mut B, addr: u32) -> u32 {
    bus.read32(addr & !3).rotate_right(8 * (addr & 3))
}

/// Halfword load with the ARM7 misalignment behavior (LDRH at an odd address
/// returns the aligned halfword rotated right by 8).
pub(crate) fn load16_rotated<B: Bus>(bus: &mut B, addr: u32) -> u32 {
    u32::from(bus.read16(addr & !1)).rotate_right(8 * (addr & 1))
}

/// Signed halfword load; at an odd address the ARM7 degrades to a signed
/// *byte* load (LDRSH quirk).
pub(crate) fn load16_signed<B: Bus>(bus: &mut B, addr: u32) -> u32 {
    if addr & 1 != 0 {
        bus.read8(addr) as i8 as i32 as u32
    } else {
        bus.read16(addr) as i16 as i32 as u32
    }
}

/// Approximate multiplier array cycles (Booth early-termination on the
/// multiplier operand).
pub(crate) fn multiplier_cycles(rs: u32) -> u64 {
    if rs & 0xFFFF_FF00 == 0 || rs & 0xFFFF_FF00 == 0xFFFF_FF00 {
        1
    } else if rs & 0xFFFF_0000 == 0 || rs & 0xFFFF_0000 == 0xFFFF_0000 {
        2
    } else if rs & 0xFF00_0000 == 0 || rs & 0xFF00_0000 == 0xFF00_0000 {
        3
    } else {
        4
    }
}
