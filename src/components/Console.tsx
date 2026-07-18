import { RefObject } from "react";

interface Props {
  canvasRef: RefObject<HTMLCanvasElement>;
  fileName: string;
  fps: number;
  onEject: () => void;
}

const CONTROLS: [string, string][] = [
  ["D-Pad", "Arrow keys"],
  ["A / B", "X / Z"],
  ["L / R", "A / S"],
  ["Start / Select", "Enter / Backspace"],
];

export function Console({ canvasRef, fileName, fps, onEject }: Props) {
  return (
    <main className="stage">
      <div className="console">
        <div className="console__top">
          <span className="cart-name">{fileName}</span>
          <span className="fps" title="frames per second">
            {fps} fps
          </span>
        </div>

        <div className="screen">
          <canvas ref={canvasRef} className="screen__canvas" />
        </div>

        <div className="console__bottom">
          <div className="dot dot--a" />
          <div className="dot dot--b" />
          <div className="brand">POCKET</div>
          <button className="btn btn--eject" onClick={onEject}>
            ⏏ Eject
          </button>
        </div>
      </div>

      <aside className="controls">
        <h3>Controls</h3>
        <ul>
          {CONTROLS.map(([label, keys]) => (
            <li key={label}>
              <span>{label}</span>
              <kbd>{keys}</kbd>
            </li>
          ))}
        </ul>
      </aside>
    </main>
  );
}
