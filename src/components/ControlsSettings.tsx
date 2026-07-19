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
