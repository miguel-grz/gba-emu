//! Thumb (16-bit) instruction set decode & execute.
//!
//! Covers all 19 ARMv4T Thumb formats. Flag behavior, the barrel shifter and
//! misaligned-load quirks are shared with the ARM module via `cpu::mod`, so
//! the two instruction sets cannot drift apart on ALU semantics.
//!
//! ARMv4T notes honored here: `POP {PC}` and `MOV/ADD PC` do *not*
//! interwork (bit 0 is ignored, the CPU stays in Thumb) — only BX switches
//! state; BL is the two-halfword pair; the v5 BLX prefix decodes as
//! undefined.

use super::{
    add_with_carry, asr, load16_rotated, load16_signed, load32_rotated, lsl, lsr,
    multiplier_cycles, ror, Cpu, Mode, FLAG_C, FLAG_T, FLAG_V, VEC_UNDEFINED,
};
use crate::memory::Bus;

pub(super) fn execute<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u16) {
    let op = u32::from(op);
    match op >> 12 {
        0x0 | 0x1 => {
            if op & 0x1800 == 0x1800 {
                add_subtract(cpu, op); // format 2
            } else {
                shift_immediate(cpu, op); // format 1
            }
        }
        0x2 | 0x3 => immediate_op(cpu, op), // format 3
        0x4 => {
            if op & 0x0800 != 0 {
                load_pc_relative(cpu, bus, op); // format 6
            } else if op & 0x0400 != 0 {
                hi_register_op(cpu, bus, op); // format 5
            } else {
                alu_op(cpu, op); // format 4
            }
        }
        0x5 => {
            if op & 0x0200 == 0 {
                load_store_register(cpu, bus, op); // format 7
            } else {
                load_store_sign_extended(cpu, bus, op); // format 8
            }
        }
        0x6 | 0x7 => load_store_word_byte(cpu, bus, op), // format 9
        0x8 => load_store_halfword(cpu, bus, op),        // format 10
        0x9 => load_store_sp_relative(cpu, bus, op),     // format 11
        0xA => load_address(cpu, op),                    // format 12
        0xB => {
            if op & 0x0F00 == 0 {
                adjust_sp(cpu, op); // format 13
            } else if op & 0x0600 == 0x0400 {
                push_pop(cpu, bus, op); // format 14
            } else {
                undefined(cpu, bus);
            }
        }
        0xC => multiple_load_store(cpu, bus, op), // format 15
        0xD => {
            let cond = op >> 8 & 0xF;
            if cond == 0xF {
                software_interrupt(cpu, bus, op); // format 17
            } else if cond == 0xE {
                undefined(cpu, bus);
            } else {
                conditional_branch(cpu, bus, op, cond); // format 16
            }
        }
        0xE => {
            if op & 0x0800 == 0 {
                unconditional_branch(cpu, bus, op); // format 18
            } else {
                undefined(cpu, bus); // BLX suffix — ARMv5 only
            }
        }
        _ => long_branch_link(cpu, bus, op), // format 19
    }
}

/// Format 1: LSL/LSR/ASR Rd, Rs, #imm5.
fn shift_immediate(cpu: &mut Cpu, op: u32) {
    let amount = op >> 6 & 0x1F;
    let rs = (op >> 3 & 7) as usize;
    let rd = (op & 7) as usize;
    let value = cpu.reg(rs);
    let carry_in = cpu.flag_c();
    let (result, carry) = match op >> 11 & 3 {
        0 => lsl(value, amount, carry_in),
        1 => lsr(value, if amount == 0 { 32 } else { amount }, carry_in),
        _ => asr(value, if amount == 0 { 32 } else { amount }, carry_in),
    };
    cpu.set_flag(FLAG_C, carry);
    cpu.set_nz(result);
    *cpu.reg_mut(rd) = result;
}

/// Format 2: ADD/SUB Rd, Rs, Rn / #imm3.
fn add_subtract(cpu: &mut Cpu, op: u32) {
    let value = if op & 1 << 10 != 0 {
        op >> 6 & 7
    } else {
        cpu.reg((op >> 6 & 7) as usize)
    };
    let a = cpu.reg((op >> 3 & 7) as usize);
    let rd = (op & 7) as usize;
    let (result, carry, overflow) = if op & 1 << 9 != 0 {
        add_with_carry(a, !value, 1)
    } else {
        add_with_carry(a, value, 0)
    };
    cpu.set_flag(FLAG_C, carry);
    cpu.set_flag(FLAG_V, overflow);
    cpu.set_nz(result);
    *cpu.reg_mut(rd) = result;
}

/// Format 3: MOV/CMP/ADD/SUB Rd, #imm8.
fn immediate_op(cpu: &mut Cpu, op: u32) {
    let rd = (op >> 8 & 7) as usize;
    let imm = op & 0xFF;
    match op >> 11 & 3 {
        0 => {
            cpu.set_nz(imm); // MOV
            *cpu.reg_mut(rd) = imm;
        }
        1 => {
            let (r, c, v) = add_with_carry(cpu.reg(rd), !imm, 1); // CMP
            cpu.set_flag(FLAG_C, c);
            cpu.set_flag(FLAG_V, v);
            cpu.set_nz(r);
        }
        2 => {
            let (r, c, v) = add_with_carry(cpu.reg(rd), imm, 0); // ADD
            cpu.set_flag(FLAG_C, c);
            cpu.set_flag(FLAG_V, v);
            cpu.set_nz(r);
            *cpu.reg_mut(rd) = r;
        }
        _ => {
            let (r, c, v) = add_with_carry(cpu.reg(rd), !imm, 1); // SUB
            cpu.set_flag(FLAG_C, c);
            cpu.set_flag(FLAG_V, v);
            cpu.set_nz(r);
            *cpu.reg_mut(rd) = r;
        }
    }
}

/// Format 4: register-to-register ALU operations.
fn alu_op(cpu: &mut Cpu, op: u32) {
    let rs = (op >> 3 & 7) as usize;
    let rd = (op & 7) as usize;
    let a = cpu.reg(rd);
    let b = cpu.reg(rs);
    let carry_in = cpu.flag_c();

    let logical = |cpu: &mut Cpu, result: u32, write: bool| {
        cpu.set_nz(result);
        if write {
            *cpu.reg_mut(rd) = result;
        }
    };
    let arith = |cpu: &mut Cpu, x: u32, y: u32, c: u32, write: bool| {
        let (r, carry, overflow) = add_with_carry(x, y, c);
        cpu.set_flag(FLAG_C, carry);
        cpu.set_flag(FLAG_V, overflow);
        cpu.set_nz(r);
        if write {
            *cpu.reg_mut(rd) = r;
        }
    };
    let shift = |cpu: &mut Cpu, (result, carry): (u32, bool)| {
        cpu.add_cycles(1);
        cpu.set_flag(FLAG_C, carry);
        cpu.set_nz(result);
        *cpu.reg_mut(rd) = result;
    };

    match op >> 6 & 0xF {
        0x0 => logical(cpu, a & b, true),                    // AND
        0x1 => logical(cpu, a ^ b, true),                    // EOR
        0x2 => shift(cpu, lsl(a, b & 0xFF, carry_in)),       // LSL
        0x3 => shift(cpu, lsr(a, b & 0xFF, carry_in)),       // LSR
        0x4 => shift(cpu, asr(a, b & 0xFF, carry_in)),       // ASR
        0x5 => arith(cpu, a, b, u32::from(carry_in), true),  // ADC
        0x6 => arith(cpu, a, !b, u32::from(carry_in), true), // SBC
        0x7 => shift(cpu, ror(a, b & 0xFF, carry_in)),       // ROR
        0x8 => logical(cpu, a & b, false),                   // TST
        0x9 => arith(cpu, 0, !b, 1, true),                   // NEG
        0xA => arith(cpu, a, !b, 1, false),                  // CMP
        0xB => arith(cpu, a, b, 0, false),                   // CMN
        0xC => logical(cpu, a | b, true),                    // ORR
        0xD => {
            let result = a.wrapping_mul(b); // MUL (C unpredictable; unchanged)
            cpu.add_cycles(multiplier_cycles(b));
            logical(cpu, result, true);
        }
        0xE => logical(cpu, a & !b, true), // BIC
        _ => logical(cpu, !b, true),       // MVN
    }
}

/// Format 5: ADD/CMP/MOV on high registers, and BX.
fn hi_register_op<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let rd = ((op & 7) | (op >> 4 & 8)) as usize;
    let rs = (op >> 3 & 0xF) as usize;
    let value = cpu.reg(rs);
    match op >> 8 & 3 {
        0 => {
            // ADD (no flags)
            let result = cpu.reg(rd).wrapping_add(value);
            if rd == 15 {
                cpu.branch_to(bus, result); // stays Thumb; bit 0 ignored
            } else {
                *cpu.reg_mut(rd) = result;
            }
        }
        1 => {
            let (r, c, v) = add_with_carry(cpu.reg(rd), !value, 1); // CMP
            cpu.set_flag(FLAG_C, c);
            cpu.set_flag(FLAG_V, v);
            cpu.set_nz(r);
        }
        2 => {
            if rd == 15 {
                cpu.branch_to(bus, value); // MOV PC
            } else {
                *cpu.reg_mut(rd) = value;
            }
        }
        _ => {
            // BX: the one Thumb instruction that can switch to ARM state.
            cpu.set_flag(FLAG_T, value & 1 != 0);
            cpu.branch_to(bus, value);
        }
    }
}

/// Format 6: LDR Rd, [PC, #imm8*4] (PC reads word-aligned).
fn load_pc_relative<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let rd = (op >> 8 & 7) as usize;
    let addr = (cpu.reg(15) & !2).wrapping_add((op & 0xFF) * 4);
    *cpu.reg_mut(rd) = bus.read32(addr);
    cpu.add_cycles(1);
}

/// Format 7: LDR/STR/LDRB/STRB Rd, [Rb, Ro].
fn load_store_register<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let addr = cpu
        .reg((op >> 3 & 7) as usize)
        .wrapping_add(cpu.reg((op >> 6 & 7) as usize));
    let rd = (op & 7) as usize;
    match op >> 10 & 3 {
        0 => bus.write32(addr & !3, cpu.reg(rd)), // STR
        1 => bus.write8(addr, cpu.reg(rd) as u8), // STRB
        2 => {
            *cpu.reg_mut(rd) = load32_rotated(bus, addr); // LDR
            cpu.add_cycles(1);
        }
        _ => {
            *cpu.reg_mut(rd) = u32::from(bus.read8(addr)); // LDRB
            cpu.add_cycles(1);
        }
    }
}

/// Format 8: STRH/LDRH/LDSB/LDSH Rd, [Rb, Ro].
fn load_store_sign_extended<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let addr = cpu
        .reg((op >> 3 & 7) as usize)
        .wrapping_add(cpu.reg((op >> 6 & 7) as usize));
    let rd = (op & 7) as usize;
    match op >> 10 & 3 {
        0 => bus.write16(addr & !1, cpu.reg(rd) as u16), // STRH
        1 => {
            *cpu.reg_mut(rd) = bus.read8(addr) as i8 as i32 as u32; // LDSB
            cpu.add_cycles(1);
        }
        2 => {
            *cpu.reg_mut(rd) = load16_rotated(bus, addr); // LDRH
            cpu.add_cycles(1);
        }
        _ => {
            *cpu.reg_mut(rd) = load16_signed(bus, addr); // LDSH
            cpu.add_cycles(1);
        }
    }
}

/// Format 9: LDR/STR/LDRB/STRB Rd, [Rb, #imm5].
fn load_store_word_byte<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let byte = op & 1 << 12 != 0;
    let load = op & 1 << 11 != 0;
    let imm = op >> 6 & 0x1F;
    let rb = (op >> 3 & 7) as usize;
    let rd = (op & 7) as usize;
    let addr = cpu.reg(rb).wrapping_add(if byte { imm } else { imm * 4 });
    // Loads add one internal cycle; stores none. All bus accesses (including
    // the instruction fetch) are timed by the memory model.
    match (load, byte) {
        (false, false) => bus.write32(addr & !3, cpu.reg(rd)),
        (false, true) => bus.write8(addr, cpu.reg(rd) as u8),
        (true, false) => {
            *cpu.reg_mut(rd) = load32_rotated(bus, addr);
            cpu.add_cycles(1);
        }
        (true, true) => {
            *cpu.reg_mut(rd) = u32::from(bus.read8(addr));
            cpu.add_cycles(1);
        }
    }
}

/// Format 10: LDRH/STRH Rd, [Rb, #imm5*2].
fn load_store_halfword<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let load = op & 1 << 11 != 0;
    let addr = cpu
        .reg((op >> 3 & 7) as usize)
        .wrapping_add((op >> 6 & 0x1F) * 2);
    let rd = (op & 7) as usize;
    if load {
        *cpu.reg_mut(rd) = load16_rotated(bus, addr);
        cpu.add_cycles(1);
    } else {
        bus.write16(addr & !1, cpu.reg(rd) as u16);
    }
}

/// Format 11: LDR/STR Rd, [SP, #imm8*4].
fn load_store_sp_relative<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let load = op & 1 << 11 != 0;
    let rd = (op >> 8 & 7) as usize;
    let addr = cpu.reg(13).wrapping_add((op & 0xFF) * 4);
    if load {
        *cpu.reg_mut(rd) = load32_rotated(bus, addr);
        cpu.add_cycles(1);
    } else {
        bus.write32(addr & !3, cpu.reg(rd));
    }
}

/// Format 12: ADD Rd, PC/SP, #imm8*4 (PC reads word-aligned; no flags).
fn load_address(cpu: &mut Cpu, op: u32) {
    let rd = (op >> 8 & 7) as usize;
    let base = if op & 1 << 11 != 0 {
        cpu.reg(13)
    } else {
        cpu.reg(15) & !2
    };
    *cpu.reg_mut(rd) = base.wrapping_add((op & 0xFF) * 4);
}

/// Format 13: ADD SP, #±imm7*4.
fn adjust_sp(cpu: &mut Cpu, op: u32) {
    let offset = (op & 0x7F) * 4;
    let sp = cpu.reg(13);
    *cpu.reg_mut(13) = if op & 1 << 7 != 0 {
        sp.wrapping_sub(offset)
    } else {
        sp.wrapping_add(offset)
    };
}

/// Format 14: PUSH {Rlist, LR} / POP {Rlist, PC}.
fn push_pop<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let pop = op & 1 << 11 != 0;
    let lr_pc = op & 1 << 8 != 0;
    let list = op & 0xFF;

    if list == 0 && !lr_pc {
        // Empty-list quirk, mirroring ARM LDM/STM: transfers PC, SP ±0x40.
        let sp = cpu.reg(13);
        if pop {
            let value = bus.read32(sp & !3);
            *cpu.reg_mut(13) = sp.wrapping_add(0x40);
            cpu.branch_to(bus, value);
        } else {
            let addr = sp.wrapping_sub(0x40);
            *cpu.reg_mut(13) = addr;
            bus.write32(addr & !3, cpu.reg(15).wrapping_add(2));
        }
        return;
    }

    let count = list.count_ones() + u32::from(lr_pc);
    if pop {
        let mut addr = cpu.reg(13);
        *cpu.reg_mut(13) = addr.wrapping_add(4 * count);
        for i in 0..8 {
            if list & 1 << i != 0 {
                *cpu.reg_mut(i) = bus.read32(addr & !3);
                addr = addr.wrapping_add(4);
            }
        }
        if lr_pc {
            let value = bus.read32(addr & !3);
            cpu.branch_to(bus, value); // ARMv4T: no interworking, bit 0 ignored
        }
        cpu.add_cycles(1); // POP/LDM internal cycle
    } else {
        let mut addr = cpu.reg(13).wrapping_sub(4 * count);
        *cpu.reg_mut(13) = addr;
        for i in 0..8 {
            if list & 1 << i != 0 {
                bus.write32(addr & !3, cpu.reg(i));
                addr = addr.wrapping_add(4);
            }
        }
        if lr_pc {
            bus.write32(addr & !3, cpu.reg(14));
        }
        // PUSH/STM has no internal cycle.
    }
}

/// Format 15: LDMIA/STMIA Rb!, {Rlist}.
fn multiple_load_store<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let load = op & 1 << 11 != 0;
    let rb = (op >> 8 & 7) as usize;
    let list = op & 0xFF;
    let base = cpu.reg(rb);

    if list == 0 {
        // Empty-list quirk: transfers PC, base += 0x40.
        if load {
            let value = bus.read32(base & !3);
            *cpu.reg_mut(rb) = base.wrapping_add(0x40);
            cpu.branch_to(bus, value);
        } else {
            bus.write32(base & !3, cpu.reg(15).wrapping_add(2));
            *cpu.reg_mut(rb) = base.wrapping_add(0x40);
        }
        return;
    }

    let count = list.count_ones();
    let new_base = base.wrapping_add(4 * count);
    if load {
        // Writeback first; if Rb is in the list the loaded value wins.
        *cpu.reg_mut(rb) = new_base;
        let mut addr = base;
        for i in 0..8 {
            if list & 1 << i != 0 {
                *cpu.reg_mut(i) = bus.read32(addr & !3);
                addr = addr.wrapping_add(4);
            }
        }
        cpu.add_cycles(1); // LDM internal cycle
    } else {
        let first = list.trailing_zeros() as usize;
        let mut addr = base;
        for i in 0..8 {
            if list & 1 << i != 0 {
                // Base in list: first position stores the old base, later
                // positions the written-back value (matches ARM STM).
                let value = if i == rb && i != first {
                    new_base
                } else {
                    cpu.reg(i)
                };
                bus.write32(addr & !3, value);
                addr = addr.wrapping_add(4);
            }
        }
        *cpu.reg_mut(rb) = new_base;
        // STM has no internal cycle.
    }
}

/// Format 16: conditional branch (8-bit offset).
fn conditional_branch<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32, cond: u32) {
    if cpu.check_cond(cond) {
        let offset = ((((op & 0xFF) as i8) as i32) << 1) as u32;
        let target = cpu.reg(15).wrapping_add(offset);
        cpu.branch_to(bus, target);
    }
}

/// Format 17: SWI. The Thumb encoding carries the number in bits 7:0.
fn software_interrupt(cpu: &mut Cpu, bus: &mut impl Bus, op: u32) {
    cpu.do_swi(bus, (op & 0xFF) as u8);
}

/// Format 18: unconditional branch (11-bit offset).
fn unconditional_branch<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let offset = (((op & 0x7FF) << 21) as i32 >> 20) as u32;
    let target = cpu.reg(15).wrapping_add(offset);
    cpu.branch_to(bus, target);
}

/// Format 19: BL, split across two halfwords. The halves are handled
/// independently so interrupted or abused pairs still behave like hardware.
fn long_branch_link<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let imm = op & 0x7FF;
    if op & 1 << 11 == 0 {
        // First half: LR = PC + (signed imm11 << 12).
        let offset = ((imm << 21) as i32 >> 9) as u32;
        *cpu.reg_mut(14) = cpu.reg(15).wrapping_add(offset);
    } else {
        // Second half: branch to LR + (imm11 << 1); LR = return address | 1.
        let target = cpu.reg(14).wrapping_add(imm << 1);
        *cpu.reg_mut(14) = cpu.reg(15).wrapping_sub(2) | 1;
        cpu.branch_to(bus, target);
    }
}

fn undefined(cpu: &mut Cpu, bus: &mut impl Bus) {
    let lr = cpu.reg(15).wrapping_sub(2);
    cpu.enter_exception(bus, VEC_UNDEFINED, Mode::Undefined, lr);
}
