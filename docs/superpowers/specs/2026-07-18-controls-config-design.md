# Controls configuration — design spec

**Date:** 2026-07-18
**Status:** Approved, ready for implementation plan

## Goal

Let the player remap every GBA button to a keyboard key **and** to a gamepad
button, from the Settings screen, with changes persisting and applying live.
Today the keymap is a hardcoded `DEFAULT_KEYMAP` in `src/lib/gba.ts` and there
is no gamepad support; Settings shows a "Coming soon" placeholder.

Scope for this iteration: **keyboard remapping + gamepad support together**, both
fully remappable.

## Architecture (Approach A)

A dedicated input layer, separate from the emulator runner and from `App.tsx`:

```
App (controls state, loaded once)
 ├─ loadControls() on mount
 ├─→ ControlsSettings   (edit → onChange → setControls + saveControls)
 └─→ InputManager       (play → setControls on change; drives runner.setButton)
```

Three new units, each with one responsibility:

1. `src/lib/controls.ts` — the config type, defaults, persistence, and label
   helpers. No DOM, no emulator knowledge. Pure and testable.
2. `src/lib/input.ts` — `InputManager`: owns keyboard listeners and the gamepad
   polling loop, translates raw input into GBA button edges.
3. `src/components/ControlsSettings.tsx` — the interactive rebinding UI.

## 1. Data model & persistence (`src/lib/controls.ts`)

```ts
import { Button } from "./gba";

export interface Controls {
  keyboard: Record<Button, string>;   // Button → KeyboardEvent.code
  gamepad: Record<Button, number>;     // Button → Gamepad.buttons[] index
}

export const DEFAULT_CONTROLS: Controls = {
  keyboard: {
    [Button.A]: "KeyX", [Button.B]: "KeyZ",
    [Button.Select]: "Backspace", [Button.Start]: "Enter",
    [Button.Right]: "ArrowRight", [Button.Left]: "ArrowLeft",
    [Button.Up]: "ArrowUp", [Button.Down]: "ArrowDown",
    [Button.R]: "KeyS", [Button.L]: "KeyA",
  },
  gamepad: {
    [Button.A]: 1, [Button.B]: 0,          // standard layout: B=south(0), A=east(1)
    [Button.Select]: 8, [Button.Start]: 9,
    [Button.L]: 4, [Button.R]: 5,           // shoulder buttons
    [Button.Up]: 12, [Button.Down]: 13, [Button.Left]: 14, [Button.Right]: 15, // d-pad
  },
};

const STORAGE_KEY = "pocket:controls";

export function loadControls(): Controls;   // JSON.parse, deep-merged over DEFAULT_CONTROLS
export function saveControls(c: Controls): void;   // JSON.stringify to localStorage

// Human-readable labels for the UI.
export function keyLabel(code: string): string;      // "KeyX"→"X", "ArrowUp"→"↑", else the code
export function padLabel(index: number): string;     // standard-layout name, else "Button N"
```

Design notes:

- **Indexed by `Button`**, inverting today's `Record<code, Button>`. This is the
  natural shape for the rebinding UI ("for button A, which key?") and for
  showing the current binding per row.
- `loadControls()` **merges over the defaults** so a config saved before a new
  binding existed still yields a complete object (missing keys fall back to
  default). Corrupt/absent JSON → `DEFAULT_CONTROLS`.
- Plain JSON in a single localStorage key. Does **not** use `persist.ts` (that
  is for base64 binary blobs).
- `Button` is the existing numeric enum-like object in `src/lib/gba.ts`; its
  values are used as object keys.

## 2. `InputManager` (`src/lib/input.ts`)

```ts
export class InputManager {
  constructor(controls: Controls, onButton: (b: Button, pressed: boolean) => void);
  attach(): void;                    // add keydown/keyup on window + start gamepad rAF
  detach(): void;                    // remove listeners + cancelAnimationFrame
  setControls(c: Controls): void;    // hot-swap; rebuilds the reverse keyboard index
}
```

**Keyboard.** On `setControls`, build a reverse index `code → Button` from
`controls.keyboard`. In `keydown`/`keyup`, look up `e.code`; if mapped,
`e.preventDefault()` and `onButton(button, pressed)`. (Same behavior as the
current inline handler, but config-driven.)

**Gamepad.** The Gamepad API does not deliver button-state events, so poll in a
`requestAnimationFrame` loop:

1. `navigator.getGamepads()` → use the first connected pad.
2. For each `Button`, read `pad.buttons[controls.gamepad[button]]?.pressed`.
   Additionally treat the analog axes as a d-pad fallback: `axes[0] < -0.5` →
   Left, `> 0.5` → Right; `axes[1] < -0.5` → Up, `> 0.5` → Down. So both digital
   d-pads and analog sticks work.
3. Track a `Set<Button>` of currently-pressed buttons; emit `onButton` only on
   **edges** (changes vs. the previous frame), never every frame.
4. Start/stop the loop from `gamepadconnected` / `gamepaddisconnected` (and once
   at `attach()` in case a pad is already present).

**Integration in `App.tsx`.** The runner `useEffect` stops wiring listeners by
hand. Instead it creates:

```ts
const input = new InputManager(controls, (b, pressed) => {
  runner.resumeAudio();
  runner.setButton(b, pressed);
});
input.attach();
// cleanup: input.detach();
```

A separate effect calls `input.setControls(controls)` when `controls` changes,
so edits from Settings apply without restarting the game. `resumeAudio()` moves
into the callback, so the browser-audio unlock works for keyboard and for the
first gamepad press alike.

## 3. Rebinding UI (`src/components/ControlsSettings.tsx`)

Replaces the static controls table (and the "Coming soon" row) in
`src/components/Settings.tsx`. One row per GBA button, two assignable columns
(Keyboard, Gamepad), plus a gamepad status line and a reset button:

```
🎮 Gamepad connected: Xbox 360 Controller        (or "No gamepad connected")

Button      Keyboard        Gamepad
──────────────────────────────────────
A           [ X ]           [ A (East) ]
B           [ Z ]           [ B (South) ]
D-Pad ↑     [ ↑ ]           [ D-Pad ↑ ]
...
                        [ Restore defaults ]
```

**"Press to assign" interaction:**

- Click a cell → *listening* mode: cell shows "Press a key…" / "Press a
  button…" and is highlighted.
- Keyboard cell: the next `keydown` is captured and assigned; `Escape` cancels.
- Gamepad cell: while listening, a short poll assigns the first pressed button.
- **Conflict resolution:** if the chosen key/button was already bound to another
  GBA button, steal it — clear the previous owner's binding so no two GBA
  buttons share one input. (A binding can be empty; the InputManager just won't
  map it.)
- Each change calls `onChange(nextControls)` in `App`, which does
  `saveControls()` + `setControls` state update → live via `input.setControls()`.

**Labels:** `keyLabel` / `padLabel` from `controls.ts`. Standard-layout gamepad
indices show names ("A (East)", "LT", "D-Pad ↑"); unknown indices show
"Button N".

**Gamepad status:** a line above the table reacting to
`gamepadconnected`/`gamepaddisconnected`, showing the pad's `id` or "No gamepad
connected".

## Data flow summary

```
App
  controls: Controls           // useState, initial = loadControls()
  setControls(next):           // saveControls(next); setState(next)
  ├─ <Settings> → <ControlsSettings controls onChange={setControls} />
  └─ runner effect: new InputManager(controls, cb); attach()
     + effect: input.setControls(controls) on controls change
```

## Error handling

- `loadControls`: malformed/missing JSON → `DEFAULT_CONTROLS` (never throws).
- Merge guards against partial saved configs.
- Gamepad polling: `getGamepads()` can return `null` slots — guard before read.
- Rebinding a key already in use reassigns it rather than erroring
  (steal-and-clear), so the config never holds duplicate bindings.

## Testing

The Rust core does not change — no new `cargo test`. Verification is functional,
in the browser:

- Rebind a keyboard key; confirm the game responds to the new key and not the
  old one.
- Reload; confirm the custom binding persists.
- "Restore defaults" returns the table and behavior to `DEFAULT_CONTROLS`.
- Gamepad status indicator reacts to connect/disconnect.
- Gamepad button polling that requires physical hardware is validated as far as
  the environment allows; anything not exercisable on real hardware here is
  called out explicitly rather than claimed.

## Out of scope (this iteration)

- Multiple keys bound to one GBA button (one key + one gamepad button each).
- Per-game control profiles (one global config).
- Rebinding the analog-stick-as-d-pad threshold or dead zones (fixed at 0.5).
- Touch / on-screen controls.
