//! ARM7TDMI correctness tests.
//!
//! Two layers:
//!
//! 1. **Hand-assembled instruction tests** that always run in CI. Each test
//!    loads machine code into a flat RAM bus, steps an exact number of
//!    instructions and asserts on registers/flags/memory. Opcode comments
//!    show the assembly each word encodes.
//!
//! 2. **Test-ROM runs** against public-domain CPU test ROMs, executed
//!    headlessly. ROMs are not redistributed with this repo; drop them into
//!    `core/tests/roms/` and the tests pick them up (they skip, loudly, when
//!    absent):
//!      * `arm.gba` / `thumb.gba` — jsmolka's gba-tests
//!        (https://github.com/jsmolka/gba-tests). On failure these park in an
//!        infinite loop with the failing test number in r12; on success r12
//!        is 0. That convention gives us a real pass/fail assertion.
//!      * `armwrestler.gba` — ARMWrestler reports results on screen, which
//!        needs the Phase 3 PPU to render; until then we run it as a smoke
//!        test (it must keep executing without crashing) and dump state.

use gba_core::{Bus, Cpu, Memory, Mode};
use std::path::PathBuf;

/// 64 KiB of flat RAM mapped at address 0, mirrored across the address space.
/// `bios_present` lets a test select the LLE `SWI` path (exception to the
/// vector) instead of the HLE path (software emulation), which is the default.
struct RamBus {
    mem: Vec<u8>,
    bios_present: bool,
}

impl RamBus {
    fn new() -> Self {
        RamBus {
            mem: vec![0; 0x1_0000],
            bios_present: false,
        }
    }

    fn write_word(&mut self, addr: u32, value: u32) {
        self.write32(addr, value);
    }
}

impl Bus for RamBus {
    fn read8(&mut self, addr: u32) -> u8 {
        self.mem[addr as usize & 0xFFFF]
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.mem[addr as usize & 0xFFFF] = value;
    }
    fn has_bios(&self) -> bool {
        self.bios_present
    }
}

const CPSR_SYS_ARM: u32 = 0xDF; // System mode, IRQ+FIQ masked, ARM state
const CPSR_SYS_THUMB: u32 = 0xFF; // System mode, IRQ+FIQ masked, Thumb state

/// Load ARM code at address 0 and prepare a CPU in System mode.
fn arm_cpu(code: &[u32]) -> (Cpu, RamBus) {
    let mut bus = RamBus::new();
    for (i, op) in code.iter().enumerate() {
        bus.write_word(i as u32 * 4, *op);
    }
    let mut cpu = Cpu::new();
    cpu.set_cpsr(CPSR_SYS_ARM);
    cpu.jump(&mut bus, 0);
    (cpu, bus)
}

/// Load Thumb code at address 0 and prepare a CPU in System mode.
fn thumb_cpu(code: &[u16]) -> (Cpu, RamBus) {
    let mut bus = RamBus::new();
    for (i, op) in code.iter().enumerate() {
        bus.write16(i as u32 * 2, *op);
    }
    let mut cpu = Cpu::new();
    cpu.set_cpsr(CPSR_SYS_THUMB);
    cpu.jump(&mut bus, 0);
    (cpu, bus)
}

fn run(cpu: &mut Cpu, bus: &mut RamBus, steps: usize) {
    for _ in 0..steps {
        cpu.step(bus);
    }
}

// ---------------------------------------------------------------- ARM: ALU

#[test]
fn arm_adds_sets_overflow_and_negative() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3E00102, // mvn r0, #0x80000000   ; r0 = 0x7FFFFFFF
        0xE2900001, // adds r0, r0, #1       ; overflow into the sign bit
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0x8000_0000);
    assert!(cpu.flag_n() && cpu.flag_v());
    assert!(!cpu.flag_c() && !cpu.flag_z());
}

#[test]
fn arm_adds_sets_carry_and_zero() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3E00000, // mvn r0, #0            ; r0 = 0xFFFFFFFF
        0xE2900001, // adds r0, r0, #1
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0);
    assert!(cpu.flag_c() && cpu.flag_z());
    assert!(!cpu.flag_n() && !cpu.flag_v());
}

#[test]
fn arm_subs_borrow_clears_carry() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00000, // mov r0, #0
        0xE2500001, // subs r0, r0, #1
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0xFFFF_FFFF);
    assert!(cpu.flag_n());
    assert!(!cpu.flag_c()); // ARM carry = NOT borrow
}

#[test]
fn arm_immediate_rotate_sets_carry() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3B00102, // movs r0, #0x80000000  ; rotated immediate, carry = bit 31
    ]);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.reg(0), 0x8000_0000);
    assert!(cpu.flag_c() && cpu.flag_n());
}

#[test]
fn arm_lsr_32_encoded_as_zero() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A01102, // mov r1, #0x80000000
        0xE1B00021, // movs r0, r1, lsr #32  ; encoded as lsr #0
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0);
    assert!(cpu.flag_z() && cpu.flag_c());
}

#[test]
fn arm_rrx_shifts_through_carry() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A01003, // mov r1, #3
        0xE1B00061, // movs r0, r1, rrx      ; C in = 0 -> r0 = 1, C out = 1
        0xE1B02061, // movs r2, r1, rrx      ; C in = 1 -> r2 = 0x80000001
    ]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.reg(0), 1);
    assert_eq!(cpu.reg(2), 0x8000_0001);
    assert!(cpu.flag_c());
}

#[test]
fn arm_mul_and_long_multiplies() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3E00000, // mvn r0, #0            ; r0 = -1
        0xE3A01004, // mov r1, #4
        0xE0D32190, // smulls r2, r3, r0, r1 ; -1 * 4 = -4
        0xE0954190, // umulls r4, r5, r0, r1 ; 0xFFFFFFFF * 4
    ]);
    run(&mut cpu, &mut bus, 4);
    assert_eq!((cpu.reg(3), cpu.reg(2)), (0xFFFF_FFFF, 0xFFFF_FFFC));
    assert_eq!((cpu.reg(5), cpu.reg(4)), (0x0000_0003, 0xFFFF_FFFC));
}

// ------------------------------------------------------- ARM: PC / pipeline

#[test]
fn arm_pc_reads_as_plus_8() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE1A0000F, // mov r0, pc            ; at 0x00 -> reads 0x08
    ]);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.reg(0), 8);
}

#[test]
fn arm_pc_reads_as_plus_12_with_register_shift() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE1A0021F, // mov r0, pc, lsl r2    ; r2 = 0; reg-shift -> PC = +12
    ]);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.reg(0), 12);
}

#[test]
fn arm_str_pc_stores_plus_12() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A01C01, // mov r1, #0x100
        0xE581F000, // str pc, [r1]          ; at 0x04 -> stores 0x04 + 12
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(bus.read32(0x100), 0x10);
}

#[test]
fn arm_branch_and_link() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xEB000002, // bl 0x10
        0xE3A01007, // mov r1, #7            ; return lands here
        0xEAFFFFFE, // b .
        0x00000000, 0xE3A02005, // 0x10: mov r2, #5
        0xE1A0F00E, // mov pc, lr
    ]);
    run(&mut cpu, &mut bus, 4);
    assert_eq!(cpu.reg(14), 4);
    assert_eq!(cpu.reg(2), 5);
    assert_eq!(cpu.reg(1), 7);
}

// ------------------------------------------------------------- ARM: memory

#[test]
fn arm_ldr_misaligned_rotates() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A01C01, // mov r1, #0x100
        0xE3811002, // orr r1, r1, #2
        0xE5910000, // ldr r0, [r1]          ; address 0x102
    ]);
    bus.write_word(0x100, 0x1122_3344);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.reg(0), 0x3344_1122); // aligned word rotated right 16
}

#[test]
fn arm_ldrh_misaligned_rotates() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A01C01, // mov r1, #0x100
        0xE3811001, // orr r1, r1, #1
        0xE1D100B0, // ldrh r0, [r1]         ; address 0x101
    ]);
    bus.write16(0x100, 0x1122);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.reg(0), 0x2200_0011); // halfword rotated right 8
}

#[test]
fn arm_stm_base_in_list_stores_old_base_when_first() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00C01, // mov r0, #0x100
        0xE3A010AA, // mov r1, #0xAA
        0xE8A00003, // stmia r0!, {r0, r1}
    ]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(bus.read32(0x100), 0x100); // r0 first in list -> old base
    assert_eq!(bus.read32(0x104), 0xAA);
    assert_eq!(cpu.reg(0), 0x108); // writeback
}

#[test]
fn arm_ldm_loaded_base_wins_over_writeback() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00C01, // mov r0, #0x100
        0xE8B00003, // ldmia r0!, {r0, r1}
    ]);
    bus.write_word(0x100, 0xCAFE_0000);
    bus.write_word(0x104, 0xBEEF_0000);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0xCAFE_0000); // loaded value, not 0x108
    assert_eq!(cpu.reg(1), 0xBEEF_0000);
}

// ----------------------------------------------- ARM: modes and exceptions

#[test]
fn banked_sp_survives_mode_switches() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A0DA01, // mov sp, #0x1000       ; System sp
        0xE3A000D2, // mov r0, #0xD2         ; IRQ mode bits
        0xE121F000, // msr cpsr_c, r0
        0xE3A0DA02, // mov sp, #0x2000       ; IRQ sp
        0xE3A010DF, // mov r1, #0xDF         ; System mode bits
        0xE121F001, // msr cpsr_c, r1
    ]);
    run(&mut cpu, &mut bus, 6);
    assert_eq!(cpu.mode(), Mode::System);
    assert_eq!(cpu.reg(13), 0x1000); // System sp restored
                                     // Switch back to IRQ: its sp must have been preserved in the bank.
    let (mut cpu2, mut bus2) = arm_cpu(&[
        0xE3A0DA01, 0xE3A000D2, 0xE121F000, 0xE3A0DA02, 0xE3A010DF, 0xE121F001,
        0xE121F000, // msr cpsr_c, r0        ; back to IRQ
    ]);
    run(&mut cpu2, &mut bus2, 7);
    assert_eq!(cpu2.mode(), Mode::Irq);
    assert_eq!(cpu2.reg(13), 0x2000);
}

#[test]
fn swi_enters_supervisor_and_returns() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00005, // 0x00: mov r0, #5
        0xEF000000, // 0x04: swi #0
        0xE3A01007, // 0x08: mov r1, #7      ; also the SWI vector target!
    ]);
    bus.bios_present = true; // exercise the LLE exception path
                             // Vector 0x08 contains "mov r1, #7"; follow it with a return.
    bus.write_word(0x0C, 0xE1B0F00E); // movs pc, lr  (restores CPSR)
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.mode(), Mode::Supervisor);
    assert_eq!(cpu.spsr(), CPSR_SYS_ARM);
    assert_eq!(cpu.reg(14), 0x08); // return address after the SWI
    run(&mut cpu, &mut bus, 2); // handler body + movs pc, lr
    assert_eq!(cpu.mode(), Mode::System);
    assert_eq!(cpu.cpsr(), CPSR_SYS_ARM);
    assert_eq!(cpu.reg(1), 7);
}

#[test]
fn irq_taken_when_unmasked() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00005, // mov r0, #5
        0xE3A01007, // mov r1, #7
    ]);
    cpu.set_cpsr(0x5F); // System, IRQs enabled
    run(&mut cpu, &mut bus, 1);
    let pc_before = cpu.pc();
    cpu.irq(&mut bus);
    assert_eq!(cpu.mode(), Mode::Irq);
    assert_eq!(cpu.spsr(), 0x5F);
    assert_eq!(cpu.pc(), 0x18); // IRQ vector
                                // Handler convention: SUBS PC, LR, #4 resumes the interrupted flow.
    assert_eq!(cpu.reg(14).wrapping_sub(4), pc_before);
    assert_ne!(cpu.cpsr() & 0x80, 0); // I flag set on entry
}

#[test]
fn irq_masked_by_i_flag() {
    let (mut cpu, mut bus) = arm_cpu(&[0xE3A00005]);
    let before = cpu.cpsr();
    cpu.irq(&mut bus); // CPSR_SYS_ARM has I set
    assert_eq!(cpu.cpsr(), before);
    assert_eq!(cpu.mode(), Mode::System);
}

#[test]
fn undefined_instruction_traps() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE7F000F0, // undefined-instruction encoding
    ]);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.mode(), Mode::Undefined);
    assert_eq!(cpu.pc(), 0x04); // undefined vector
    assert_eq!(cpu.reg(14), 0x04); // return past the offending instruction
}

// -------------------------------------------------------- ARM/Thumb bridge

#[test]
fn bx_interworks_both_directions() {
    let (mut cpu, mut bus) = arm_cpu(&[
        0xE3A00011, // mov r0, #0x11         ; 0x10 | thumb bit
        0xE3A03020, // mov r3, #0x20
        0xE12FFF10, // bx r0                 ; -> Thumb at 0x10
    ]);
    bus.write16(0x10, 0x2205); // movs r2, #5
    bus.write16(0x12, 0x4718); // bx r3      ; -> ARM at 0x20
    bus.write_word(0x20, 0xE3A01007); // mov r1, #7
    run(&mut cpu, &mut bus, 6);
    assert!(!cpu.is_thumb());
    assert_eq!(cpu.reg(2), 5);
    assert_eq!(cpu.reg(1), 7);
}

// ------------------------------------------------------------------- Thumb

#[test]
fn thumb_add_sub_set_flags() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2000, // movs r0, #0
        0x3801, // subs r0, #1
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0xFFFF_FFFF);
    assert!(cpu.flag_n() && !cpu.flag_c());
}

#[test]
fn thumb_shift_immediate_flags() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2103, // movs r1, #3
        0x0088, // lsls r0, r1, #2
    ]);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 12);
    assert!(!cpu.flag_c() && !cpu.flag_z());
}

#[test]
fn thumb_alu_neg_and_cmp() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2105, // movs r1, #5
        0x4248, // negs r0, r1           ; r0 = -5
        0x42C8, // cmn r0, r1            ; -5 + 5 = 0
    ]);
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.reg(0), 5u32.wrapping_neg());
    assert!(cpu.flag_z() && cpu.flag_c());
}

#[test]
fn thumb_pc_relative_load_aligns_pc() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2000, // 0x0: movs r0, #0
        0x4801, // 0x2: ldr r0, [pc, #4] ; (pc=6 & !2) + 4 = 0x8
        0xE7FE, // 0x4: b .
        0x0000, // 0x6: (padding)
    ]);
    bus.write_word(0x8, 0xDEAD_BEEF);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.reg(0), 0xDEAD_BEEF);
}

#[test]
fn thumb_hi_register_add_reads_pc() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x4479, // add r1, pc            ; r1 = 0 + (0x0 + 4)
    ]);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.reg(1), 4);
}

#[test]
fn thumb_push_pop_roundtrip() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2503, // movs r5, #3
        0xB420, // push {r5}
        0x2500, // movs r5, #0
        0xBC20, // pop {r5}
    ]);
    cpu.set_reg(13, 0x8000);
    run(&mut cpu, &mut bus, 4);
    assert_eq!(cpu.reg(5), 3);
    assert_eq!(cpu.reg(13), 0x8000);
}

#[test]
fn thumb_bl_pair_links_and_returns() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0xF000, // 0x0: bl prefix
        0xF806, // 0x2: bl suffix -> 0x10
        0x2107, // 0x4: movs r1, #7      ; return lands here
        0xE7FE, // 0x6: b .
        0x0000, 0x0000, 0x0000, 0x0000, 0x2205, // 0x10: movs r2, #5
        0x4770, // 0x12: bx lr
    ]);
    run(&mut cpu, &mut bus, 5);
    assert_eq!(cpu.reg(2), 5);
    assert_eq!(cpu.reg(1), 7);
    assert_eq!(cpu.reg(14), 0x05); // return address | thumb bit
    assert!(cpu.is_thumb());
}

#[test]
fn thumb_conditional_branch() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2800, // 0x0: cmp r0, #0
        0xD101, // 0x2: bne +2           ; not taken (r0 == 0)
        0x2101, // 0x4: movs r1, #1
        0x2202, // 0x6: movs r2, #2
    ]);
    run(&mut cpu, &mut bus, 4);
    assert_eq!(cpu.reg(1), 1); // fallthrough executed
    assert_eq!(cpu.reg(2), 2);
}

#[test]
fn thumb_swi_enters_arm_supervisor() {
    let (mut cpu, mut bus) = thumb_cpu(&[
        0x2001, // 0x0: movs r0, #1
        0xDF00, // 0x2: swi #0
    ]);
    bus.bios_present = true; // exercise the LLE exception path
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.mode(), Mode::Supervisor);
    assert!(!cpu.is_thumb()); // exceptions execute in ARM state
    assert_eq!(cpu.reg(14), 0x4); // next Thumb instruction
    assert_eq!(cpu.spsr(), CPSR_SYS_THUMB);
}

// ------------------------------------------------- Memory map integration

#[test]
fn synthetic_rom_runs_from_cartridge_space() {
    // A miniature "ROM": executes from 0x08000000, stores to IWRAM and
    // EWRAM, reads one back, then parks in an idle loop.
    let code: [u32; 8] = [
        0xE3A00301, // mov r0, #0x04000000   ; (io base, just to form addresses)
        0xE3A01403, // mov r1, #0x03000000   ; IWRAM
        0xE3A02402, // mov r2, #0x02000000   ; EWRAM
        0xE3A03042, // mov r3, #0x42
        0xE5813000, // str r3, [r1]
        0xE5823000, // str r3, [r2]
        0xE5914000, // ldr r4, [r1]
        0xEAFFFFFE, // b .
    ];
    let mut rom = Vec::new();
    for op in code {
        rom.extend_from_slice(&op.to_le_bytes());
    }
    let mut mem = Memory::new(rom).expect("rom fits");
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut mem);
    assert_eq!(cpu.pc(), 0x0800_0000);
    assert_eq!(cpu.mode(), Mode::System);
    for _ in 0..8 {
        cpu.step(&mut mem);
    }
    assert_eq!(cpu.reg(4), 0x42);
    assert_eq!(mem.read8(0x0200_0000), 0x42);
    assert_eq!(cpu.pc(), 0x0800_001C); // parked in the idle loop
}

#[test]
fn rom_too_large_is_rejected() {
    let err = Memory::new(vec![0; 33 * 1024 * 1024])
        .map(|_| ())
        .unwrap_err();
    assert!(err.to_string().contains("exceeding"));
}

// -------------------------------------------------------------- Test ROMs

fn rom_path(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/roms")
        .join(name);
    path.exists().then_some(path)
}

/// Run a ROM headlessly until it parks in a branch-to-self loop (the idle
/// convention of CPU test ROMs) or the step budget runs out.
fn run_rom(path: &std::path::Path, max_steps: u64) -> (Cpu, Memory, u64) {
    let mut mem = Memory::from_files(path, None).expect("failed to load ROM");
    let mut cpu = Cpu::new();
    cpu.skip_bios(&mut mem);
    let mut steps = 0;
    while steps < max_steps {
        let pc = cpu.pc();
        let cycles = cpu.step(&mut mem);
        mem.tick(cycles); // advance the PPU so display timing progresses
        steps += 1;
        if cpu.pc() == pc {
            break; // tight infinite loop: the ROM is done (pass or fail)
        }
    }
    (cpu, mem, steps)
}

fn dump(cpu: &Cpu) -> String {
    let regs: Vec<String> = (0..16)
        .map(|i| format!("r{i}={:08X}", cpu.reg(i)))
        .collect();
    format!(
        "pc={:08X} mode={:?} thumb={} cpsr={:08X}\n{}",
        cpu.pc(),
        cpu.mode(),
        cpu.is_thumb(),
        cpu.cpsr(),
        regs.join(" ")
    )
}

fn jsmolka_rom_passes(name: &str) {
    let Some(path) = rom_path(name) else {
        eprintln!("SKIPPED: put jsmolka's {name} in core/tests/roms/ to run this test");
        return;
    };
    let (cpu, _mem, steps) = run_rom(&path, 50_000_000);
    assert!(
        steps < 50_000_000,
        "{name} never reached an end-of-test loop\n{}",
        dump(&cpu)
    );
    // jsmolka convention: failed test number in r12, 0 on full pass.
    assert_eq!(
        cpu.reg(12),
        0,
        "{name} reports failing test #{} \n{}",
        cpu.reg(12),
        dump(&cpu)
    );
}

#[test]
fn rom_jsmolka_arm() {
    jsmolka_rom_passes("arm.gba");
}

#[test]
fn rom_jsmolka_thumb() {
    jsmolka_rom_passes("thumb.gba");
}

#[test]
fn rom_armwrestler_smoke() {
    let Some(path) = rom_path("armwrestler.gba") else {
        eprintln!("SKIPPED: put armwrestler.gba in core/tests/roms/ to run this smoke test");
        return;
    };
    // ARMWrestler shows results on screen; verifying them needs the Phase 3
    // PPU. Until then: it must run a healthy number of instructions without
    // trapping into the undefined handler and staying there.
    let (cpu, _mem, steps) = run_rom(&path, 5_000_000);
    eprintln!("armwrestler ran {steps} steps\n{}", dump(&cpu));
    assert_ne!(
        cpu.mode(),
        Mode::Undefined,
        "parked in undefined trap\n{}",
        dump(&cpu)
    );
}
