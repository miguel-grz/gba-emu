import { useRef, useState } from "react";
import { GameMeta } from "../lib/library";
import { GameCard } from "./GameCard";
import { GbaConsole } from "./GbaConsole";
import { IconSearch } from "./icons";
import type { Section } from "./Sidebar";

interface Props {
  section: Section;
  ready: boolean;
  games: GameMeta[];
  busy: string | null;
  error: string | null;
  search: string;
  onSearch: (q: string) => void;
  onAdd: (file: File) => void;
  onPlay: (name: string) => void;
  onToggleFav: (name: string, favorite: boolean) => void;
  onRemove: (name: string) => void;
}

const HERO: Record<Exclude<Section, "settings">, { title: string; sub: string }> = {
  library: { title: "Library", sub: "All your Game Boy Advance games in one place." },
  favorites: { title: "Favorites", sub: "The games you keep coming back to." },
  recents: { title: "Recently played", sub: "Jump back into where you left off." },
};

const EMPTY: Record<Exclude<Section, "settings">, string> = {
  library: "Drop a .gba file anywhere here, or use Add ROM. Your games stay in this browser.",
  favorites: "No favorites yet. Tap the heart on any game to pin it here.",
  recents: "Nothing played yet. Launch a game and it’ll show up here.",
};

export function Library(props: Props) {
  const { section, ready, games, busy, error, search, onSearch, onAdd } = props;
  const hero = HERO[section as Exclude<Section, "settings">] ?? HERO.library;
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    for (const file of Array.from(e.dataTransfer.files)) onAdd(file);
  };

  return (
    <section
      className="view"
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
      style={dragging ? { outline: "2px dashed var(--violet-bright)", outlineOffset: 8, borderRadius: 20 } : undefined}
    >
      <div className="hero">
        <div style={{ position: "relative", zIndex: 2 }}>
          <div className="hero__eyebrow">Pocket</div>
          <h1 className="hero__title">{hero.title}</h1>
          <p className="hero__sub">{hero.sub}</p>
        </div>
        <GbaConsole className="hero__art" />
      </div>

      <div className="toolbar">
        <span className="section-title">
          {games.length} {games.length === 1 ? "game" : "games"}
        </span>
        <div className="search">
          <IconSearch />
          <input
            value={search}
            placeholder="Search games…"
            onChange={(e) => onSearch(e.target.value)}
          />
        </div>
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
        <div className="empty">
          <GbaConsole />
          <h3>{ready ? (section === "library" ? "Your library is empty" : "Nothing here yet") : "Warming up…"}</h3>
          <p>{EMPTY[section as Exclude<Section, "settings">] ?? EMPTY.library}</p>
        </div>
      ) : (
        <div className="grid">
          {busy && section === "library" && (
            <div className="card card--busy">
              <div className="card__art">
                <div className="spinner" />
              </div>
              <div className="card__body">
                <div className="card__title">Importing {busy}…</div>
              </div>
            </div>
          )}
          {games.map((g) => (
            <GameCard
              key={g.name}
              game={g}
              onPlay={props.onPlay}
              onToggleFav={props.onToggleFav}
              onRemove={props.onRemove}
            />
          ))}
        </div>
      )}
    </section>
  );
}
