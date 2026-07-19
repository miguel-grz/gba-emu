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
