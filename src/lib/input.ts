// Owns all live game input: keyboard events plus a gamepad polling loop (the
// Gamepad API has no button-state events, so button states must be sampled each
// frame). Emits only edges — a button change, not a per-frame repeat — to the
// onButton callback, which the app wires to runner.setButton.

import { Button } from "./gba";
import { ALL_BUTTONS, Controls } from "./controls";

const AXIS_THRESHOLD = 0.5;

export class InputManager {
  private controls: Controls;
  private readonly onButton: (b: Button, pressed: boolean) => void;
  private keyIndex = new Map<string, Button>();
  private padPressed = new Set<Button>();
  private rafId = 0;
  private running = false;

  constructor(controls: Controls, onButton: (b: Button, pressed: boolean) => void) {
    this.controls = controls;
    this.onButton = onButton;
    this.rebuildKeyIndex();
  }

  attach(): void {
    window.addEventListener("keydown", this.onKeyDown);
    window.addEventListener("keyup", this.onKeyUp);
    this.running = true;
    this.rafId = requestAnimationFrame(this.poll);
  }

  detach(): void {
    window.removeEventListener("keydown", this.onKeyDown);
    window.removeEventListener("keyup", this.onKeyUp);
    this.running = false;
    if (this.rafId) cancelAnimationFrame(this.rafId);
    this.rafId = 0;
    // Release anything the gamepad was holding so it doesn't stick.
    for (const b of this.padPressed) this.onButton(b, false);
    this.padPressed.clear();
  }

  setControls(c: Controls): void {
    this.controls = c;
    this.rebuildKeyIndex();
  }

  private rebuildKeyIndex(): void {
    this.keyIndex.clear();
    for (const b of ALL_BUTTONS) {
      const code = this.controls.keyboard[b];
      if (code) this.keyIndex.set(code, b);
    }
  }

  private onKeyDown = (e: KeyboardEvent) => {
    if (e.repeat) return;
    const b = this.keyIndex.get(e.code);
    if (b === undefined) return;
    e.preventDefault();
    this.onButton(b, true);
  };

  private onKeyUp = (e: KeyboardEvent) => {
    const b = this.keyIndex.get(e.code);
    if (b === undefined) return;
    e.preventDefault();
    this.onButton(b, false);
  };

  private poll = () => {
    if (!this.running) return;
    const pads = navigator.getGamepads?.() ?? [];
    const pad = pads.find((p): p is Gamepad => p !== null) ?? null;
    if (pad) {
      const now = new Set<Button>();
      for (const b of ALL_BUTTONS) {
        const idx = this.controls.gamepad[b];
        if (idx !== undefined && idx >= 0 && pad.buttons[idx]?.pressed) now.add(b);
      }
      // Analog stick as a d-pad fallback, so a stick works even without a
      // digital d-pad binding.
      const ax = pad.axes[0] ?? 0;
      const ay = pad.axes[1] ?? 0;
      if (ax <= -AXIS_THRESHOLD) now.add(Button.Left);
      if (ax >= AXIS_THRESHOLD) now.add(Button.Right);
      if (ay <= -AXIS_THRESHOLD) now.add(Button.Up);
      if (ay >= AXIS_THRESHOLD) now.add(Button.Down);
      // Emit only edges vs. the previous frame.
      for (const b of now) if (!this.padPressed.has(b)) this.onButton(b, true);
      for (const b of this.padPressed) if (!now.has(b)) this.onButton(b, false);
      this.padPressed = now;
    }
    this.rafId = requestAnimationFrame(this.poll);
  };
}
