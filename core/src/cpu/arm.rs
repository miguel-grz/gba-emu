//! ARM (32-bit) instruction set decode & execute.
//!
//! Decoding is a mask/match cascade ordered so that the specific bit patterns
//! (BX, PSR transfers, multiplies, SWP, halfword transfers) are recognized
//! before the broad data-processing class that would otherwise swallow them.
//! ARM7-specific edge cases that test ROMs exercise are implemented:
//! misaligned load rotation, `STR PC` storing PC+12, LDM/STM base-in-list and
//! empty-register-list behavior, and user-bank transfers via the S bit.

use super::{
    add_with_carry, asr, load16_rotated, load16_signed, load32_rotated, lsl, lsr,
    multiplier_cycles, ror, rrx, Cpu, Mode, FLAG_C, FLAG_T, FLAG_V, VEC_UNDEFINED,
};
use crate::memory::Bus;

pub(super) fn execute<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    // Order matters: specific patterns first.
    if op & 0x0FFF_FFF0 == 0x012F_FF10 {
        branch_exchange(cpu, bus, op);
    } else if op & 0x0FBF_0FFF == 0x010F_0000 {
        mrs(cpu, op);
    } else if op & 0x0FB0_FFF0 == 0x0120_F000 {
        msr(cpu, op, false);
    } else if op & 0x0FB0_F000 == 0x0320_F000 {
        msr(cpu, op, true);
    } else if op & 0x0FC0_00F0 == 0x0000_0090 {
        multiply(cpu, op);
    } else if op & 0x0F80_00F0 == 0x0080_0090 {
        multiply_long(cpu, op);
    } else if op & 0x0FB0_0FF0 == 0x0100_0090 {
        swap(cpu, bus, op);
    } else if op & 0x0E00_0090 == 0x0000_0090 {
        halfword_transfer(cpu, bus, op);
    } else if op & 0x0C00_0000 == 0x0000_0000 {
        data_processing(cpu, bus, op);
    } else if op & 0x0E00_0010 == 0x0600_0010 {
        // Register-offset load/store with bit 4 set: the canonical ARMv4
        // undefined-instruction encoding.
        undefined(cpu, bus);
    } else if op & 0x0C00_0000 == 0x0400_0000 {
        single_transfer(cpu, bus, op);
    } else if op & 0x0E00_0000 == 0x0800_0000 {
        block_transfer(cpu, bus, op);
    } else if op & 0x0E00_0000 == 0x0A00_0000 {
        branch(cpu, bus, op);
    } else if op & 0x0F00_0000 == 0x0F00_0000 {
        software_interrupt(cpu, bus, op);
    } else {
        // Coprocessor instructions: the GBA has no coprocessors, so these
        // take the undefined-instruction trap, as on hardware.
        undefined(cpu, bus);
    }
}

fn branch(cpu: &mut Cpu, bus: &mut impl Bus, op: u32) {
    // Sign-extend the 24-bit offset and scale by 4 in one shift pair.
    let offset = ((op << 8) as i32 >> 6) as u32;
    if op & 1 << 24 != 0 {
        // BL: LR = address of the next instruction (PC visible as +8, so -4).
        *cpu.reg_mut(14) = cpu.reg(15).wrapping_sub(4);
    }
    let target = cpu.reg(15).wrapping_add(offset);
    cpu.branch_to(bus, target);
}

fn branch_exchange(cpu: &mut Cpu, bus: &mut impl Bus, op: u32) {
    let value = cpu.reg((op & 0xF) as usize);
    cpu.set_flag(FLAG_T, value & 1 != 0);
    cpu.branch_to(bus, value);
}

fn data_processing<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let opcode = op >> 21 & 0xF;
    let set_flags = op & 1 << 20 != 0;
    // TST/TEQ/CMP/CMN without S are the PSR-transfer encodings; anything that
    // reaches here in that shape is architecturally unpredictable — ignore.
    if !set_flags && (0x8..=0xB).contains(&opcode) {
        return;
    }
    let rn = (op >> 16 & 0xF) as usize;
    let rd = (op >> 12 & 0xF) as usize;
    let carry_in = cpu.flag_c();

    let (operand2, shifter_carry, rn_val) = if op & 1 << 25 != 0 {
        // Immediate: 8-bit value rotated right by twice the rotate field.
        let rotate = (op >> 8 & 0xF) * 2;
        let value = (op & 0xFF).rotate_right(rotate);
        let carry = if rotate == 0 {
            carry_in
        } else {
            value >> 31 != 0
        };
        (value, carry, cpu.reg(rn))
    } else {
        let rm = (op & 0xF) as usize;
        let by_register = op & 1 << 4 != 0;
        // With a register-specified shift the PC prefetches one further step:
        // R15 reads as instruction address + 12 instead of + 8.
        let pc_adjust = if by_register { 4 } else { 0 };
        let adj = |i: usize, v: u32| {
            if i == 15 {
                v.wrapping_add(pc_adjust)
            } else {
                v
            }
        };
        let rm_val = adj(rm, cpu.reg(rm));
        let rn_val = adj(rn, cpu.reg(rn));
        let shift_type = op >> 5 & 3;
        let (value, carry) = if by_register {
            cpu.add_cycles(1); // internal cycle for the register shift
            let amount = cpu.reg((op >> 8 & 0xF) as usize) & 0xFF;
            match shift_type {
                0 => lsl(rm_val, amount, carry_in),
                1 => lsr(rm_val, amount, carry_in),
                2 => asr(rm_val, amount, carry_in),
                _ => ror(rm_val, amount, carry_in),
            }
        } else {
            let amount = op >> 7 & 0x1F;
            match shift_type {
                0 => lsl(rm_val, amount, carry_in),
                1 => lsr(rm_val, if amount == 0 { 32 } else { amount }, carry_in),
                2 => asr(rm_val, if amount == 0 { 32 } else { amount }, carry_in),
                _ if amount == 0 => rrx(rm_val, carry_in),
                _ => ror(rm_val, amount, carry_in),
            }
        };
        (value, carry, rn_val)
    };

    let mut carry = shifter_carry;
    let mut overflow = cpu.flag_v();
    let carry_bit = u32::from(carry_in);
    let mut write_result = true;
    let result = match opcode {
        0x0 => rn_val & operand2, // AND
        0x1 => rn_val ^ operand2, // EOR
        0x2 => {
            let (r, c, v) = add_with_carry(rn_val, !operand2, 1); // SUB
            carry = c;
            overflow = v;
            r
        }
        0x3 => {
            let (r, c, v) = add_with_carry(operand2, !rn_val, 1); // RSB
            carry = c;
            overflow = v;
            r
        }
        0x4 => {
            let (r, c, v) = add_with_carry(rn_val, operand2, 0); // ADD
            carry = c;
            overflow = v;
            r
        }
        0x5 => {
            let (r, c, v) = add_with_carry(rn_val, operand2, carry_bit); // ADC
            carry = c;
            overflow = v;
            r
        }
        0x6 => {
            let (r, c, v) = add_with_carry(rn_val, !operand2, carry_bit); // SBC
            carry = c;
            overflow = v;
            r
        }
        0x7 => {
            let (r, c, v) = add_with_carry(operand2, !rn_val, carry_bit); // RSC
            carry = c;
            overflow = v;
            r
        }
        0x8 => {
            write_result = false; // TST
            rn_val & operand2
        }
        0x9 => {
            write_result = false; // TEQ
            rn_val ^ operand2
        }
        0xA => {
            let (r, c, v) = add_with_carry(rn_val, !operand2, 1); // CMP
            carry = c;
            overflow = v;
            write_result = false;
            r
        }
        0xB => {
            let (r, c, v) = add_with_carry(rn_val, operand2, 0); // CMN
            carry = c;
            overflow = v;
            write_result = false;
            r
        }
        0xC => rn_val | operand2,  // ORR
        0xD => operand2,           // MOV
        0xE => rn_val & !operand2, // BIC
        _ => !operand2,            // MVN
    };

    if set_flags {
        if rd == 15 {
            // <op>S with Rd = PC: exception return — restore CPSR from SPSR.
            let spsr = cpu.spsr();
            cpu.set_cpsr(spsr);
        } else {
            cpu.set_flag(FLAG_C, carry);
            cpu.set_flag(FLAG_V, overflow);
            cpu.set_nz(result);
        }
    }
    if write_result {
        if rd == 15 {
            // branch_to honors the (possibly just-restored) T bit.
            cpu.branch_to(bus, result);
        } else {
            *cpu.reg_mut(rd) = result;
        }
    }
}

fn mrs(cpu: &mut Cpu, op: u32) {
    let rd = (op >> 12 & 0xF) as usize;
    let value = if op & 1 << 22 != 0 {
        cpu.spsr()
    } else {
        cpu.cpsr()
    };
    *cpu.reg_mut(rd) = value;
}

fn msr(cpu: &mut Cpu, op: u32, immediate: bool) {
    let value = if immediate {
        (op & 0xFF).rotate_right((op >> 8 & 0xF) * 2)
    } else {
        cpu.reg((op & 0xF) as usize)
    };
    let mut mask = 0u32;
    for (bit, field) in [(16, 0xFF), (17, 0xFF00), (18, 0xFF_0000), (19, 0xFF00_0000)] {
        if op & 1 << bit != 0 {
            mask |= field;
        }
    }
    if op & 1 << 22 != 0 {
        // SPSR write; no SPSR exists in User/System (set_spsr ignores it).
        let new = (cpu.spsr() & !mask) | (value & mask);
        cpu.set_spsr(new);
    } else {
        // User mode may only touch the flag field.
        if !cpu.mode().is_privileged() {
            mask &= 0xF000_0000;
        }
        let current = cpu.cpsr();
        let mut new = (current & !mask) | (value & mask);
        // Switching ARM/Thumb state via MSR is unpredictable on hardware;
        // keep the current T bit.
        new = (new & !FLAG_T) | (current & FLAG_T);
        cpu.set_cpsr(new);
    }
}

fn multiply(cpu: &mut Cpu, op: u32) {
    let rd = (op >> 16 & 0xF) as usize;
    let rn = (op >> 12 & 0xF) as usize;
    let rs = (op >> 8 & 0xF) as usize;
    let rm = (op & 0xF) as usize;
    let mut result = cpu.reg(rm).wrapping_mul(cpu.reg(rs));
    if op & 1 << 21 != 0 {
        result = result.wrapping_add(cpu.reg(rn)); // MLA
        cpu.add_cycles(1);
    }
    cpu.add_cycles(multiplier_cycles(cpu.reg(rs)));
    *cpu.reg_mut(rd) = result;
    if op & 1 << 20 != 0 {
        // MULS: N and Z are set; C is architecturally meaningless on ARM7
        // (we leave it unchanged).
        cpu.set_nz(result);
    }
}

fn multiply_long(cpu: &mut Cpu, op: u32) {
    let rd_hi = (op >> 16 & 0xF) as usize;
    let rd_lo = (op >> 12 & 0xF) as usize;
    let rs = (op >> 8 & 0xF) as usize;
    let rm = (op & 0xF) as usize;
    let signed = op & 1 << 22 != 0;
    let a = cpu.reg(rm);
    let b = cpu.reg(rs);
    let mut result: u64 = if signed {
        (i64::from(a as i32) * i64::from(b as i32)) as u64
    } else {
        u64::from(a) * u64::from(b)
    };
    if op & 1 << 21 != 0 {
        // UMLAL/SMLAL: accumulate onto RdHi:RdLo.
        let acc = (u64::from(cpu.reg(rd_hi)) << 32) | u64::from(cpu.reg(rd_lo));
        result = result.wrapping_add(acc);
        cpu.add_cycles(1);
    }
    cpu.add_cycles(multiplier_cycles(b) + 1);
    *cpu.reg_mut(rd_lo) = result as u32;
    *cpu.reg_mut(rd_hi) = (result >> 32) as u32;
    if op & 1 << 20 != 0 {
        cpu.set_flag(super::FLAG_N, result >> 63 != 0);
        cpu.set_flag(super::FLAG_Z, result == 0);
    }
}

fn swap<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let rn = (op >> 16 & 0xF) as usize;
    let rd = (op >> 12 & 0xF) as usize;
    let rm = (op & 0xF) as usize;
    let addr = cpu.reg(rn);
    // Read-before-write makes Rd == Rm == Rn behave like hardware.
    if op & 1 << 22 != 0 {
        let old = u32::from(bus.read8(addr)); // SWPB
        bus.write8(addr, cpu.reg(rm) as u8);
        *cpu.reg_mut(rd) = old;
    } else {
        let old = load32_rotated(bus, addr); // SWP
        bus.write32(addr & !3, cpu.reg(rm));
        *cpu.reg_mut(rd) = old;
    }
    cpu.add_cycles(1); // 1 internal cycle; the 3 bus accesses are timed by memory
}

fn single_transfer<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let pre_index = op & 1 << 24 != 0;
    let up = op & 1 << 23 != 0;
    let byte = op & 1 << 22 != 0;
    let writeback = op & 1 << 21 != 0;
    let load = op & 1 << 20 != 0;
    let rn = (op >> 16 & 0xF) as usize;
    let rd = (op >> 12 & 0xF) as usize;

    let offset = if op & 1 << 25 != 0 {
        // Shifted register offset (immediate shift amounts only).
        let rm_val = cpu.reg((op & 0xF) as usize);
        let amount = op >> 7 & 0x1F;
        let carry_in = cpu.flag_c();
        let (value, _) = match op >> 5 & 3 {
            0 => lsl(rm_val, amount, carry_in),
            1 => lsr(rm_val, if amount == 0 { 32 } else { amount }, carry_in),
            2 => asr(rm_val, if amount == 0 { 32 } else { amount }, carry_in),
            _ if amount == 0 => rrx(rm_val, carry_in),
            _ => ror(rm_val, amount, carry_in),
        };
        value
    } else {
        op & 0xFFF
    };

    let base = cpu.reg(rn);
    let offset_addr = if up {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };
    let addr = if pre_index { offset_addr } else { base };
    // Post-indexed transfers always write back (the W bit then selects the
    // ARM7 "user-mode access" variant, irrelevant without an MMU).
    let do_writeback = !pre_index || writeback;

    if load {
        let value = if byte {
            u32::from(bus.read8(addr))
        } else {
            load32_rotated(bus, addr)
        };
        // Writeback happens first; a load into the base register wins.
        if do_writeback && rn != 15 {
            *cpu.reg_mut(rn) = offset_addr;
        }
        cpu.add_cycles(1); // LDR internal cycle (data access timed by memory)
        if rd == 15 {
            cpu.branch_to(bus, value);
        } else {
            *cpu.reg_mut(rd) = value;
        }
    } else {
        // STR of PC stores instruction address + 12 (prefetch quirk).
        let value = if rd == 15 {
            cpu.reg(15).wrapping_add(4)
        } else {
            cpu.reg(rd)
        };
        if byte {
            bus.write8(addr, value as u8);
        } else {
            bus.write32(addr & !3, value);
        }
        if do_writeback && rn != 15 {
            *cpu.reg_mut(rn) = offset_addr;
        }
        // STR has no internal cycle; its two bus accesses are timed by memory.
    }
}

fn halfword_transfer<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let pre_index = op & 1 << 24 != 0;
    let up = op & 1 << 23 != 0;
    let immediate = op & 1 << 22 != 0;
    let writeback = op & 1 << 21 != 0;
    let load = op & 1 << 20 != 0;
    let rn = (op >> 16 & 0xF) as usize;
    let rd = (op >> 12 & 0xF) as usize;
    let sh = op >> 5 & 3;
    if sh == 0 {
        // Bits 6:5 == 00 here is a leftover multiply-class pattern that the
        // earlier decode arms did not claim: unpredictable, ignore.
        return;
    }

    let offset = if immediate {
        (op >> 4 & 0xF0) | (op & 0xF)
    } else {
        cpu.reg((op & 0xF) as usize)
    };
    let base = cpu.reg(rn);
    let offset_addr = if up {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };
    let addr = if pre_index { offset_addr } else { base };
    let do_writeback = !pre_index || writeback;

    if load {
        let value = match sh {
            1 => load16_rotated(bus, addr),           // LDRH
            2 => bus.read8(addr) as i8 as i32 as u32, // LDRSB
            _ => load16_signed(bus, addr),            // LDRSH
        };
        if do_writeback && rn != 15 {
            *cpu.reg_mut(rn) = offset_addr;
        }
        cpu.add_cycles(1); // load internal cycle (data access timed by memory)
        if rd == 15 {
            cpu.branch_to(bus, value);
        } else {
            *cpu.reg_mut(rd) = value;
        }
    } else {
        if sh == 1 {
            // STRH; SH = 2/3 stores are ARMv5 doubleword ops, unpredictable
            // on ARM7 — ignored.
            let value = if rd == 15 {
                cpu.reg(15).wrapping_add(4)
            } else {
                cpu.reg(rd)
            };
            bus.write16(addr & !1, value as u16);
        }
        if do_writeback && rn != 15 {
            *cpu.reg_mut(rn) = offset_addr;
        }
        // STRH has no internal cycle.
    }
}

fn block_transfer<B: Bus>(cpu: &mut Cpu, bus: &mut B, op: u32) {
    let pre_index = op & 1 << 24 != 0;
    let up = op & 1 << 23 != 0;
    let s_bit = op & 1 << 22 != 0;
    let writeback = op & 1 << 21 != 0;
    let load = op & 1 << 20 != 0;
    let rn = (op >> 16 & 0xF) as usize;
    let mut list = op & 0xFFFF;

    let base = cpu.reg(rn);
    // ARM7 empty-list quirk: transfers R15 alone, but steps the base by 0x40
    // (as if all 16 registers were transferred).
    let empty = list == 0;
    if empty {
        list = 1 << 15;
    }
    let count = if empty { 16 } else { list.count_ones() };
    let size = 4 * count;

    // Transfers always run from the lowest address upward; decrement modes
    // just start lower.
    let start = match (pre_index, up) {
        (false, true) => base,                                     // IA
        (true, true) => base.wrapping_add(4),                      // IB
        (false, false) => base.wrapping_sub(size).wrapping_add(4), // DA
        (true, false) => base.wrapping_sub(size),                  // DB
    };
    let new_base = if up {
        base.wrapping_add(size)
    } else {
        base.wrapping_sub(size)
    };
    // S bit: user-bank transfer, except LDM with R15 in the list where it
    // means "also restore CPSR from SPSR".
    let user_bank = s_bit && !(load && list & 1 << 15 != 0);

    if load {
        // Writeback first; a load into the base register wins.
        if writeback && rn != 15 {
            *cpu.reg_mut(rn) = new_base;
        }
        let mut addr = start;
        for i in 0..16 {
            if list & 1 << i == 0 {
                continue;
            }
            let value = bus.read32(addr & !3);
            if i == 15 {
                if s_bit {
                    let spsr = cpu.spsr();
                    cpu.set_cpsr(spsr);
                }
                cpu.branch_to(bus, value);
            } else if user_bank {
                cpu.set_reg_user(i, value);
            } else {
                *cpu.reg_mut(i) = value;
            }
            addr = addr.wrapping_add(4);
        }
        cpu.add_cycles(1); // LDM internal cycle; the count accesses are timed by memory
    } else {
        let first = list.trailing_zeros() as usize;
        let mut addr = start;
        for i in 0..16 {
            if list & 1 << i == 0 {
                continue;
            }
            let value = if i == 15 {
                cpu.reg(15).wrapping_add(4) // PC stores as +12
            } else if i == rn && writeback && i != first {
                // Base in list: only the first-stored register sees the old
                // base; later positions store the written-back value.
                new_base
            } else if user_bank {
                cpu.reg_user(i)
            } else {
                cpu.reg(i)
            };
            bus.write32(addr & !3, value);
            addr = addr.wrapping_add(4);
        }
        if writeback && rn != 15 {
            *cpu.reg_mut(rn) = new_base;
        }
        // STM has no internal cycle; its accesses are timed by memory.
    }
}

fn software_interrupt(cpu: &mut Cpu, bus: &mut impl Bus, op: u32) {
    // The BIOS reads the SWI number from bits 23:16 of the ARM encoding.
    let number = (op >> 16) as u8;
    cpu.do_swi(bus, number);
}

fn undefined(cpu: &mut Cpu, bus: &mut impl Bus) {
    let lr = cpu.reg(15).wrapping_sub(4);
    cpu.enter_exception(bus, VEC_UNDEFINED, Mode::Undefined, lr);
}
