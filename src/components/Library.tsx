import { useRef, useState } from "react";
import { GameMeta } from "../lib/library";
import { GameCard } from "./GameCard";

interface Props {
  ready: boolean;
  games: GameMeta[];
  busy: string | null; // name of a game currently being imported
  error: string | null;
  onAdd: (file: File) => void;
  onPlay: (name: string) => void;
  onRemove: (name: string) => void;
}

export function Library({ ready, games, busy, error, onAdd, onPlay, onRemove }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    for (const file of Array.from(e.dataTransfer.files)) onAdd(file);
  };

  return (
    <main
      className={`library ${dragging ? "library--drag" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      <div className="library__head">
        <h2>Your library</h2>
        <button className="btn" disabled={!ready} onClick={() => inputRef.current?.click()}>
          + Add ROM
        </button>
        <input
          ref={inputRef}
          type="file"
          accept=".gba,.bin"
          hidden
          multiple
          onChange={(e) => {
            for (const file of Array.from(e.target.files ?? [])) onAdd(file);
            e.target.value = "";
          }}
        />
      </div>

      {error && <p className="error">⚠ {error}</p>}

      {games.length === 0 && !busy ? (
        <div className="library__empty">
          <div className="library__empty-art" aria-hidden>
            🕹️
          </div>
          <h3>{ready ? "Drop a ROM to get started" : "Warming up…"}</h3>
          <p>
            Drag <code>.gba</code> files anywhere here, or use “Add ROM”. Your games
            stay in this browser.
          </p>
        </div>
      ) : (
        <div className="grid">
          {busy && (
            <div className="card card--busy">
              <div className="card__art">
                <div className="spinner" />
              </div>
              <div className="card__name">Importing {busy}…</div>
            </div>
          )}
          {games.map((g) => (
            <GameCard key={g.name} game={g} onPlay={onPlay} onRemove={onRemove} />
          ))}
        </div>
      )}
    </main>
  );
}
