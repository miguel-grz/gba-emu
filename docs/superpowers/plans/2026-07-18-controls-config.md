# Controls Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the player remap every GBA button to a keyboard key and a gamepad button from Settings, persisted and applied live.

**Architecture:** A dedicated input layer sits between raw browser input and the emulator runner. `controls.ts` holds the config, defaults, persistence, and label helpers (pure). `InputManager` (`input.ts`) owns keyboard listeners and a `requestAnimationFrame` gamepad poll, emitting button *edges* to a callback. `App` keeps the config in state, feeds it to a live `InputManager` while playing and to an interactive `ControlsSettings` UI for editing.

**Tech Stack:** React 18 + TypeScript, Vite. Web Gamepad API, `localStorage`. No new dependencies.

## Global Constraints

- **No new dependencies.** Everything uses the browser platform + existing React/TS.
- **`npx tsc --noEmit` must pass** after every task (project is TS strict; this is the type gate).
- **This project has no TS unit-test runner** (by design — see spec). Verification is `tsc` + in-browser behavior, never invented test frameworks.
- **The Rust core does not change.** No `cargo` work, no `npm run wasm`.
- **localStorage key:** `pocket:controls`. Plain JSON (not the base64 `persist.ts` path).
- **`Button`** (`src/lib/gba.ts`) is a numeric const object: `A:0, B:1, Select:2, Start:3, Right:4, Left:5, Up:6, Down:7, R:8, L:9`. `Record<Button, …>` therefore has numeric keys; iterate with `ALL_BUTTONS`.
- **Commit messages must NOT include a `Co-Authored-By: Claude` trailer** (user preference).
- Dev server runs via the preview tool as `gba-emu`; verify in the browser tab, do not launch servers with Bash.

---

## File structure

- `src/lib/controls.ts` — **create.** Config type, defaults, load/save, `ALL_BUTTONS`, `BUTTON_META`, `keyLabel`, `padLabel`.
- `src/lib/input.ts` — **create.** `InputManager` class.
- `src/components/ControlsSettings.tsx` — **create.** Interactive rebinding UI.
- `src/components/Settings.tsx` — **modify.** Accept controls props; render `ControlsSettings` in place of the static table + "Coming soon" row.
- `src/App.tsx` — **modify.** Controls state + persistence; drive `InputManager` from the runner effect; pass props to `Settings`.
- `src/lib/gba.ts` — **modify.** Remove the now-unused `DEFAULT_KEYMAP`.
- `src/index.css` — **modify.** Styles for the assignable cells, listening state, and status line.

---

## Task 1: Controls config module

**Files:**
- Create: `src/lib/controls.ts`

**Interfaces:**
- Consumes: `Button` from `src/lib/gba.ts`.
- Produces:
  - `interface Controls { keyboard: Record<Button, string>; gamepad: Record<Button, number> }`
  - `const DEFAULT_CONTROLS: Controls`
  - `const ALL_BUTTONS: Button[]`
  - `const BUTTON_META: { button: Button; label: string }[]`
  - `function loadControls(): Controls`
  - `function saveControls(c: Controls): void`
  - `function keyLabel(code: string): string`
  - `function padLabel(index: number): string`

- [ ] **Step 1: Write the module**

Create `src/lib/controls.ts`:

```ts
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
```

- [ ] **Step 2: Typecheck**

Run: `cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit`
Expected: no output (passes). The module is not imported anywhere yet, so this only proves it compiles.

- [ ] **Step 3: Commit**

```bash
git add src/lib/controls.ts
git commit -m "Add controls config module (bindings, defaults, persistence)"
```

---

## Task 2: InputManager

**Files:**
- Create: `src/lib/input.ts`

**Interfaces:**
- Consumes: `Button` from `src/lib/gba.ts`; `Controls`, `ALL_BUTTONS` from `src/lib/controls.ts`.
- Produces:
  - `class InputManager`
    - `constructor(controls: Controls, onButton: (b: Button, pressed: boolean) => void)`
    - `attach(): void`
    - `detach(): void`
    - `setControls(c: Controls): void`

- [ ] **Step 1: Write the class**

Create `src/lib/input.ts`:

```ts
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
        if (idx !== undefined && pad.buttons[idx]?.pressed) now.add(b);
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
```

- [ ] **Step 2: Typecheck**

Run: `cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit`
Expected: no output. If `p is Gamepad` narrowing errors, confirm `lib.dom` types are present (they are via Vite's default `tsconfig`).

- [ ] **Step 3: Commit**

```bash
git add src/lib/input.ts
git commit -m "Add InputManager for keyboard + gamepad input"
```

---

## Task 3: Wire InputManager into the runner and hold controls state

Replaces the inline keyboard listeners in `App.tsx`'s runner effect with an `InputManager`, adds the controls state + persistence, and removes the dead `DEFAULT_KEYMAP`. After this task the game is driven through the new input layer with default bindings (no UI yet).

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/lib/gba.ts` (remove `DEFAULT_KEYMAP`)

**Interfaces:**
- Consumes: `InputManager` (Task 2); `Controls`, `loadControls`, `saveControls` (Task 1).
- Produces (used by Task 4): App passes `controls: Controls` and `onControlsChange: (c: Controls) => void` to `<Settings>`.

- [ ] **Step 1: Remove the dead default keymap**

In `src/lib/gba.ts`, delete the `DEFAULT_KEYMAP` export (lines 56–68, the block starting `/** Default keyboard → GBA mapping. */` through the closing `};`). Leave the `Button` const and everything else intact.

- [ ] **Step 2: Update App imports**

In `src/App.tsx`, change the `./lib/gba` import to drop `DEFAULT_KEYMAP`:

```ts
import { ensureWasm, generateThumbnail, GbaRunner } from "./lib/gba";
```

Add two imports after the existing `./lib/*` imports:

```ts
import { Controls, loadControls, saveControls } from "./lib/controls";
import { InputManager } from "./lib/input";
```

- [ ] **Step 3: Add controls state, a persisting setter, and an InputManager ref**

In `App()`, next to the other `useState`/`useRef` hooks (near `runnerRef`), add:

```ts
const [controls, setControls] = useState<Controls>(() => loadControls());
const inputRef = useRef<InputManager | null>(null);

const updateControls = useCallback((next: Controls) => {
  saveControls(next);
  setControls(next);
}, []);
```

- [ ] **Step 4: Replace the inline key handlers in the runner effect**

In the `useEffect(() => { … }, [playing])` runner effect, delete this block:

```ts
    const onKey = (pressed: boolean) => (e: KeyboardEvent) => {
      const button = DEFAULT_KEYMAP[e.code];
      if (button === undefined) return;
      e.preventDefault();
      runner.resumeAudio();
      runner.setButton(button, pressed);
    };
    const down = onKey(true);
    const up = onKey(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
```

and replace it with:

```ts
    const input = new InputManager(controls, (button, pressed) => {
      runner.resumeAudio();
      runner.setButton(button, pressed);
    });
    input.attach();
    inputRef.current = input;
```

Then in the same effect's cleanup `return () => { … }`, delete:

```ts
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
```

and add:

```ts
      input.detach();
      inputRef.current = null;
```

Leave the effect's dependency array as `[playing]`. The effect intentionally captures the current `controls` only at game start; live edits are pushed by the next step, so the game never restarts on a rebind. (If the linter flags the `controls` dependency, keep `[playing]` — restarting the emulator on every keystroke rebind would be the bug.)

- [ ] **Step 5: Push live control changes to the running InputManager**

Add a new effect after the runner effect:

```ts
  // Apply control edits to a running game without restarting it.
  useEffect(() => {
    inputRef.current?.setControls(controls);
  }, [controls]);
```

- [ ] **Step 6: Pass controls to Settings**

In the render, change the Settings branch from `<Settings />` to:

```tsx
        ) : section === "settings" ? (
          <Settings controls={controls} onControlsChange={updateControls} />
```

(Task 4 updates `Settings` to accept these props. `tsc` will error here until then — that is expected within this task and resolved by Step 7.)

- [ ] **Step 7: Temporarily accept props in Settings so the app compiles**

So Task 3 is independently testable, give `Settings` a minimal signature now (Task 4 replaces the body). In `src/components/Settings.tsx`, change the function signature:

```tsx
import { Controls } from "../lib/controls";

interface Props {
  controls: Controls;
  onControlsChange: (next: Controls) => void;
}

export function Settings({ controls, onControlsChange }: Props) {
```

Add `void controls; void onControlsChange;` as the first line of the function body to avoid unused-parameter errors, and leave the rest of the existing JSX unchanged for now.

- [ ] **Step 8: Typecheck**

Run: `cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit`
Expected: no output.

- [ ] **Step 9: Verify in the browser**

Ensure the preview is running (`preview_start` name `gba-emu`), open the tab, load a game (the existing library has one bootable ROM). Confirm with the keyboard: Arrow keys move, X/Z act as A/B — i.e. input still works through the new `InputManager`. Check `read_console_messages` (onlyErrors) shows no new errors.
Expected: game responds to keyboard exactly as before.

- [ ] **Step 10: Commit**

```bash
git add src/App.tsx src/lib/gba.ts src/components/Settings.tsx
git commit -m "Drive game input through InputManager with configurable bindings"
```

---

## Task 4: Rebinding UI and styles

Adds the interactive `ControlsSettings` component, mounts it in `Settings`, and styles it. After this task the player can rebind keyboard and gamepad from Settings, changes persist and apply live, and a reset restores defaults.

**Files:**
- Create: `src/components/ControlsSettings.tsx`
- Modify: `src/components/Settings.tsx`
- Modify: `src/index.css`

**Interfaces:**
- Consumes: `Controls`, `DEFAULT_CONTROLS`, `BUTTON_META`, `ALL_BUTTONS`, `keyLabel`, `padLabel` (Task 1); `Button` (`gba.ts`); the `controls` / `onControlsChange` props wired in Task 3.

- [ ] **Step 1: Write the ControlsSettings component**

Create `src/components/ControlsSettings.tsx`:

```tsx
import { useEffect, useState } from "react";
import { Button } from "../lib/gba";
import {
  ALL_BUTTONS,
  BUTTON_META,
  Controls,
  DEFAULT_CONTROLS,
  keyLabel,
  padLabel,
} from "../lib/controls";

interface Props {
  controls: Controls;
  onChange: (next: Controls) => void;
}

type Slot = "keyboard" | "gamepad";
type Listening = { button: Button; slot: Slot } | null;

export function ControlsSettings({ controls, onChange }: Props) {
  const [listening, setListening] = useState<Listening>(null);
  const [padName, setPadName] = useState<string | null>(null);

  // Reflect gamepad connect/disconnect in the status line.
  useEffect(() => {
    const scan = () => {
      const pad = (navigator.getGamepads?.() ?? []).find((p) => p !== null) ?? null;
      setPadName(pad ? pad.id : null);
    };
    scan();
    window.addEventListener("gamepadconnected", scan);
    window.addEventListener("gamepaddisconnected", scan);
    return () => {
      window.removeEventListener("gamepadconnected", scan);
      window.removeEventListener("gamepaddisconnected", scan);
    };
  }, []);

  // Assign a binding, stealing it from any other button that held it.
  const assign = (button: Button, slot: Slot, value: string | number) => {
    const next: Controls = {
      keyboard: { ...controls.keyboard },
      gamepad: { ...controls.gamepad },
    };
    if (slot === "keyboard") {
      for (const b of ALL_BUTTONS) if (next.keyboard[b] === value) delete next.keyboard[b];
      next.keyboard[button] = value as string;
    } else {
      for (const b of ALL_BUTTONS) if (next.gamepad[b] === value) delete next.gamepad[b];
      next.gamepad[button] = value as number;
    }
    onChange(next);
  };

  // Capture the next key press while listening on a keyboard cell.
  useEffect(() => {
    if (!listening || listening.slot !== "keyboard") return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      if (e.code !== "Escape") assign(listening.button, "keyboard", e.code);
      setListening(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listening]);

  // Poll for the first pressed gamepad button while listening on a gamepad cell.
  useEffect(() => {
    if (!listening || listening.slot !== "gamepad") return;
    let raf = 0;
    const poll = () => {
      const pad = (navigator.getGamepads?.() ?? []).find((p) => p !== null) ?? null;
      if (pad) {
        const idx = pad.buttons.findIndex((btn) => btn.pressed);
        if (idx >= 0) {
          assign(listening.button, "gamepad", idx);
          setListening(null);
          return;
        }
      }
      raf = requestAnimationFrame(poll);
    };
    raf = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listening]);

  const cell = (button: Button, slot: Slot, text: string) => {
    const active = listening?.button === button && listening.slot === slot;
    return (
      <button
        className={`bind ${active ? "is-listening" : ""}`}
        onClick={() => setListening(active ? null : { button, slot })}
      >
        {active ? "Press…" : text}
      </button>
    );
  };

  return (
    <div className="panel controls">
      <h3>Controls</h3>
      <div className="controls__status">
        {padName ? `🎮 ${padName}` : "No gamepad connected"}
      </div>
      {BUTTON_META.map(({ button, label }) => (
        <div className="row" key={button}>
          <span>{label}</span>
          <div className="bind-group">
            {cell(button, "keyboard", keyLabel(controls.keyboard[button] ?? ""))}
            {cell(
              button,
              "gamepad",
              controls.gamepad[button] === undefined
                ? "—"
                : padLabel(controls.gamepad[button]),
            )}
          </div>
        </div>
      ))}
      <div className="controls__actions">
        <button className="btn btn--ghost" onClick={() => onChange(DEFAULT_CONTROLS)}>
          Restore defaults
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Mount ControlsSettings in Settings**

Rewrite `src/components/Settings.tsx` so the Controls panel is the interactive component and the "About" panel stays. Replace the whole file with:

```tsx
import { Controls } from "../lib/controls";
import { ControlsSettings } from "./ControlsSettings";

interface Props {
  controls: Controls;
  onControlsChange: (next: Controls) => void;
}

export function Settings({ controls, onControlsChange }: Props) {
  return (
    <section className="settings view">
      <div className="hero" style={{ minHeight: 140 }}>
        <div style={{ position: "relative", zIndex: 2 }}>
          <div className="hero__eyebrow">Pocket</div>
          <h1 className="hero__title">Settings</h1>
        </div>
      </div>

      <ControlsSettings controls={controls} onChange={onControlsChange} />

      <div className="panel">
        <h3>About</h3>
        <div className="row">
          <span>Emulator core</span>
          <span>Rust → WebAssembly</span>
        </div>
        <div className="row">
          <span>Your games</span>
          <span>Stored in this browser</span>
        </div>
        <div className="row">
          <span>BIOS</span>
          <span>High-level (none required)</span>
        </div>
        <div className="row">
          <span>Cover art</span>
          <a
            className="link"
            href="https://thumbnails.libretro.com"
            target="_blank"
            rel="noreferrer"
          >
            thumbnails.libretro.com
          </a>
        </div>
      </div>
    </section>
  );
}
```

- [ ] **Step 3: Add styles**

In `src/index.css`, after the `.settings .row:first-of-type { … }` rule (around line 638), add:

```css
.controls__status {
  font-size: 13px;
  font-weight: 600;
  color: var(--faint);
  padding-bottom: 12px;
}

.bind-group {
  display: flex;
  gap: 8px;
}

.bind {
  min-width: 76px;
  font-family: inherit;
  font-weight: 700;
  font-size: 12px;
  color: var(--ink);
  background: rgba(150, 130, 255, 0.12);
  border: 1px solid var(--line);
  border-bottom-width: 2px;
  border-radius: 7px;
  padding: 4px 10px;
  cursor: pointer;
  transition: border-color 0.15s ease, color 0.15s ease, background 0.15s ease;
}

.bind:hover {
  border-color: var(--violet);
  color: var(--ink);
}

.bind.is-listening {
  color: #fff;
  background: var(--violet);
  border-color: var(--violet-bright);
  animation: bind-pulse 1s ease-in-out infinite;
}

@keyframes bind-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(139, 92, 246, 0.5); }
  50% { box-shadow: 0 0 0 4px rgba(139, 92, 246, 0); }
}

.controls__actions {
  margin-top: 16px;
  display: flex;
  justify-content: flex-end;
}
```

Note: the `.btn--ghost` class already exists in this file (used for the "Restore defaults" button) — do **not** redefine it. The tokens used above (`--faint`, `--ink`, `--violet`, `--violet-bright`, `--line`) are all defined at the top of the file. There is no `--text` token — use `--ink` for bright text.

- [ ] **Step 4: Typecheck**

Run: `cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit`
Expected: no output.

- [ ] **Step 5: Verify in the browser**

Open the preview tab, go to Settings:
1. The Controls panel lists all 10 buttons with a Keyboard and a Gamepad cell each, plus a "No gamepad connected" status line and a "Restore defaults" button.
2. Click the **A → Keyboard** cell; it shows "Press…" and pulses. Press `C`. The cell now reads `C`.
3. Load the game; confirm `C` acts as A and `X` no longer does (X was stolen only if reassigned — here X is still A's? No: rebinding A to C leaves X unbound for A, so X does nothing). Verify A responds to `C`.
4. Rebind A back or click **Restore defaults**; confirm the table returns to X/Z/etc.
5. Reload the page, return to Settings; confirm a custom binding you set persists.
6. Check `read_console_messages` (onlyErrors): no new errors.

Report explicitly that live gamepad button polling could not be exercised without physical hardware if no pad is available; the status line and keyboard paths are fully verified.

- [ ] **Step 6: Commit**

```bash
git add src/components/ControlsSettings.tsx src/components/Settings.tsx src/index.css
git commit -m "Add interactive controls rebinding UI in Settings"
```

---

## Self-review notes

- **Spec coverage:** data model + persistence + labels → Task 1; InputManager keyboard + gamepad poll + edge detection + analog fallback → Task 2; App integration + live `setControls` + `resumeAudio` in callback → Task 3; rebinding UI, press-to-assign, conflict steal, status line, reset → Task 4. All spec sections map to a task.
- **Out-of-scope items** (multiple keys per button, per-game profiles, dead-zone config, touch) are not implemented — matches the spec.
- **Type consistency:** `Controls`, `ALL_BUTTONS`, `BUTTON_META`, `keyLabel`, `padLabel`, `InputManager.setControls` names are identical across tasks. `Button` treated as numeric key throughout; iteration always via `ALL_BUTTONS`.
- **Gamepad hardware caveat** is surfaced in Task 4 verification rather than claimed as passing.
