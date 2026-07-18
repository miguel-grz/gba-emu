//! I/O register file (address range `0x0400_0000..0x0400_03FF`).
//!
//! Phase 2 implements the *system* registers that exist independently of the
//! PPU/APU/DMA/timers — the interrupt controller (`IE`/`IF`/`IME`), waitstate
//! control (`WAITCNT`), and the halt/post-boot flags (`HALTCNT`/`POSTFLG`).
//! Registers owned by peripherals that do not exist yet (DISPCNT, the timer
//! and DMA blocks, sound, …) are backed by plain storage so software can read
//! back what it wrote without crashing; their *behavior* arrives with the
//! peripheral that owns them.
//!
//! `KEYINPUT` reads as "no keys pressed" until input handling arrives. The
//! display registers (DISPCNT/DISPSTAT/VCOUNT/…) live in [`crate::ppu`], which
//! [`crate::memory::Memory`] routes to directly; they are not handled here.

/// Interrupt source bits, shared by `IE` and `IF`.
pub mod irq {
    pub const VBLANK: u16 = 1 << 0;
    pub const HBLANK: u16 = 1 << 1;
    pub const VCOUNT: u16 = 1 << 2;
    pub const TIMER0: u16 = 1 << 3;
    pub const TIMER1: u16 = 1 << 4;
    pub const TIMER2: u16 = 1 << 5;
    pub const TIMER3: u16 = 1 << 6;
    pub const SERIAL: u16 = 1 << 7;
    pub const DMA0: u16 = 1 << 8;
    pub const DMA1: u16 = 1 << 9;
    pub const DMA2: u16 = 1 << 10;
    pub const DMA3: u16 = 1 << 11;
    pub const KEYPAD: u16 = 1 << 12;
    pub const GAMEPAK: u16 = 1 << 13;
}

pub struct Io {
    ie: u16,
    iff: u16,
    ime: bool,
    waitcnt: u16,
    postflg: u8,
    /// Set when software writes `HALTCNT` (or the Halt SWI runs under HLE).
    /// The CPU polls and clears this to enter its halted state.
    halt_request: bool,
    /// KEYINPUT (0x130): active-low button state; 0x03FF = nothing pressed.
    keyinput: u16,
    /// Generic backing store for not-yet-implemented registers, so
    /// read-after-write works. Indexed by (offset & 0x3FF) >> 1.
    scratch: [u16; 0x200],
}

impl Io {
    pub fn new() -> Self {
        Io {
            ie: 0,
            iff: 0,
            ime: false,
            waitcnt: 0,
            postflg: 0,
            halt_request: false,
            keyinput: 0x03FF,
            scratch: [0; 0x200],
        }
    }

    /// Set the keypad state. `pressed` is active-high (bit set = held) using
    /// the KEYINPUT bit order (A, B, Select, Start, Right, Left, Up, Down,
    /// R, L); it is stored active-low as the hardware register expects.
    pub fn set_keys(&mut self, pressed: u16) {
        self.keyinput = !pressed & 0x03FF;
    }

    pub fn waitcnt(&self) -> u16 {
        self.waitcnt
    }

    /// IRQ line as seen by the CPU: an enabled interrupt is pending *and* the
    /// master enable is set. (Halt-wake on hardware ignores IME, a subtlety
    /// we accept until the scheduler lands in Phase 6.)
    pub fn irq_pending(&self) -> bool {
        self.ime && (self.ie & self.iff) != 0
    }

    /// Raise interrupt request bit(s). Peripherals call this in later phases;
    /// tests use it to drive the interrupt path today.
    pub fn raise_irq(&mut self, bits: u16) {
        self.iff |= bits;
    }

    /// Take (and clear) a pending halt request.
    pub fn take_halt_request(&mut self) -> bool {
        std::mem::replace(&mut self.halt_request, false)
    }

    pub fn read8(&mut self, offset: u32) -> u8 {
        let half = self.read16(offset & !1);
        if offset & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn read16(&mut self, offset: u32) -> u16 {
        match offset & 0x3FE {
            0x0130 => self.keyinput,
            0x0200 => self.ie,
            0x0202 => self.iff,
            0x0204 => self.waitcnt,
            0x0208 => u16::from(self.ime),
            0x0300 => u16::from(self.postflg),
            _ => self.scratch[((offset & 0x3FF) >> 1) as usize],
        }
    }

    pub fn read32(&mut self, offset: u32) -> u32 {
        u32::from(self.read16(offset)) | u32::from(self.read16(offset + 2)) << 16
    }

    pub fn write8(&mut self, offset: u32, value: u8) {
        // HALTCNT lives in the high byte of 0x0300; a byte write there must
        // not disturb POSTFLG in the low byte.
        match offset & 0x3FF {
            0x0300 => self.postflg = value & 1,
            0x0301 => self.halt_request = true,
            _ => {
                let half = self.read16(offset & !1);
                let merged = if offset & 1 == 0 {
                    (half & 0xFF00) | u16::from(value)
                } else {
                    (half & 0x00FF) | (u16::from(value) << 8)
                };
                self.write16(offset & !1, merged);
            }
        }
    }

    pub fn write16(&mut self, offset: u32, value: u16) {
        match offset & 0x3FE {
            0x0200 => self.ie = value,
            // IF is write-1-to-acknowledge: writing a 1 clears that request.
            0x0202 => self.iff &= !value,
            0x0204 => self.waitcnt = value,
            0x0208 => self.ime = value & 1 != 0,
            0x0300 => {
                self.postflg = (value & 1) as u8;
                if value & 0x8000 != 0 {
                    self.halt_request = true;
                }
            }
            // KEYINPUT is read-only.
            0x0130 => {}
            other => self.scratch[(other >> 1) as usize] = value,
        }
    }

    pub fn write32(&mut self, offset: u32, value: u32) {
        self.write16(offset, value as u16);
        self.write16(offset + 2, (value >> 16) as u16);
    }
}

impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}
