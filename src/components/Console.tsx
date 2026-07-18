import { RefObject } from "react";

interface Props {
  canvasRef: RefObject<HTMLCanvasElement>;
  fileName: string;
  fps: number;
  flash: string;
  onEject: () => void;
  onSave: () => void;
  onLoad: () => void;
}

const CONTROLS: [string, string][] = [
  ["D-Pad", "Arrows"],
  ["A / B", "X / Z"],
  ["L / R", "A / S"],
  ["Start", "Enter"],
  ["Select", "Backspace"],
];

export function Console({ canvasRef, fileName, fps, flash, onEject, onSave, onLoad }: Props) {
  return (
    <section className="player view">
      <div className="player__bar">
        <button className="btn btn--ghost" style={{ flex: "none" }} onClick={onEject}>
          ‹ Library
        </button>
        <h2>{fileName.replace(/\.(gba|bin)$/i, "")}</h2>
        <span className="player__fps">{fps} fps</span>
      </div>

      <div className="player__stage">
        <div className="screen">
          <canvas ref={canvasRef} className="screen__canvas" />
          {flash && <div className="flash">{flash}</div>}
        </div>

        <div className="dock">
          <div className="panel">
            <h3>Save state</h3>
            <div className="state-buttons">
              <button className="btn btn--ghost" onClick={onSave}>
                Save
              </button>
              <button className="btn btn--ghost" onClick={onLoad}>
                Load
              </button>
            </div>
          </div>

          <div className="panel">
            <h3>Controls</h3>
            <ul className="keys">
              {CONTROLS.map(([label, keys]) => (
                <li key={label}>
                  <span>{label}</span>
                  <kbd>{keys}</kbd>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>
    </section>
  );
}
