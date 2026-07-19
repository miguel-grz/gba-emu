// Player-configurable input bindings: which keyboard key and which gamepad
// button maps to each GBA button. Indexed by Button (the KEYINPUT bit), which
// is the natural shape for the rebinding UI ("for A, which key?"). Persisted as
// plain JSON in localStorage, merged over the defaults on load so a config
// saved before a binding existed still yields a complete object.

import { Button } from "./gba";

export interface Controls {
  keyboard: Record<Button, string>; // Button → KeyboardEvent.code
  gamepad: Record<Button, number>; // Button → Gamepad.buttons[] index
}

export const DEFAULT_CONTROLS: Controls = {
  keyboard: {
    [Button.A]: "KeyX",
    [Button.B]: "KeyZ",
    [Button.Select]: "Backspace",
    [Button.Start]: "Enter",
    [Button.Right]: "ArrowRight",
    [Button.Left]: "ArrowLeft",
    [Button.Up]: "ArrowUp",
    [Button.Down]: "ArrowDown",
    [Button.R]: "KeyS",
    [Button.L]: "KeyA",
  },
  gamepad: {
    // "standard" gamepad layout: south=0, east=1, shoulders 4/5, d-pad 12–15.
    // GBA A sits on the right (east=1), B below it (south=0) — Nintendo layout.
    [Button.A]: 1,
    [Button.B]: 0,
    [Button.Select]: 8,
    [Button.Start]: 9,
    [Button.L]: 4,
    [Button.R]: 5,
    [Button.Up]: 12,
    [Button.Down]: 13,
    [Button.Left]: 14,
    [Button.Right]: 15,
  },
};

// Every GBA button, with a display label, in the order the UI lists them.
export const BUTTON_META: { button: Button; label: string }[] = [
  { button: Button.A, label: "A" },
  { button: Button.B, label: "B" },
  { button: Button.L, label: "L" },
  { button: Button.R, label: "R" },
  { button: Button.Up, label: "D-Pad ↑" },
  { button: Button.Down, label: "D-Pad ↓" },
  { button: Button.Left, label: "D-Pad ←" },
  { button: Button.Right, label: "D-Pad →" },
  { button: Button.Start, label: "Start" },
  { button: Button.Select, label: "Select" },
];

export const ALL_BUTTONS: Button[] = BUTTON_META.map((m) => m.button);

const STORAGE_KEY = "pocket:controls";

export function loadControls(): Controls {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_CONTROLS;
    const saved = JSON.parse(raw) as Partial<Controls>;
    return {
      keyboard: { ...DEFAULT_CONTROLS.keyboard, ...(saved.keyboard ?? {}) },
      gamepad: { ...DEFAULT_CONTROLS.gamepad, ...(saved.gamepad ?? {}) },
    };
  } catch {
    return DEFAULT_CONTROLS;
  }
}

export function saveControls(c: Controls): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(c));
}

// "KeyX" → "X", "ArrowUp" → "↑", "Digit1" → "1", else the raw code.
export function keyLabel(code: string): string {
  const arrows: Record<string, string> = {
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
  };
  if (code in arrows) return arrows[code];
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return code;
}

// Standard-layout gamepad button name, else "Button N".
export function padLabel(index: number): string {
  const names: Record<number, string> = {
    0: "A (South)",
    1: "B (East)",
    2: "X (West)",
    3: "Y (North)",
    4: "LB",
    5: "RB",
    6: "LT",
    7: "RT",
    8: "Select",
    9: "Start",
    10: "L3",
    11: "R3",
    12: "D-Pad ↑",
    13: "D-Pad ↓",
    14: "D-Pad ←",
    15: "D-Pad →",
    16: "Guide",
  };
  return names[index] ?? `Button ${index}`;
}
