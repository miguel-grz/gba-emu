import { useRef, useState } from "react";

interface Props {
  ready: boolean;
  error: string | null;
  onLoad: (file: File) => void;
}

export function Landing({ ready, error, onLoad }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);

  const pick = () => inputRef.current?.click();

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    const file = e.dataTransfer.files[0];
    if (file) onLoad(file);
  };

  return (
    <main className="landing">
      <div
        className={`dropzone ${dragging ? "dropzone--over" : ""}`}
        onClick={pick}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
        role="button"
        tabIndex={0}
      >
        <div className="dropzone__art" aria-hidden>
          🎮
        </div>
        <h2>{ready ? "Drop a ROM to play" : "Warming up…"}</h2>
        <p>Drag a <code>.gba</code> file here, or click to browse.</p>
        <button className="btn" disabled={!ready} onClick={(e) => { e.stopPropagation(); pick(); }}>
          Choose a ROM
        </button>
        <input
          ref={inputRef}
          type="file"
          accept=".gba,.bin"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onLoad(file);
          }}
        />
      </div>

      {error && <p className="error">⚠ {error}</p>}

      <p className="hint">
        No BIOS needed. Try a public-domain homebrew like jsmolka's{" "}
        <code>hello.gba</code>.
      </p>
    </main>
  );
}
