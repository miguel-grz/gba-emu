import { useEffect, useRef, useState } from "react";
import { cleanFilename } from "../lib/gamedb";
import { GameMeta } from "../lib/library";
import { IconEdit, IconHeart } from "./icons";

interface Props {
  game: GameMeta;
  onPlay: (name: string) => void;
  onToggleFav: (name: string, favorite: boolean) => void;
  onRemove: (name: string) => void;
  onRename: (name: string, title: string) => void;
}

function sizeLabel(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.round(bytes / 1024)} KB`;
}

export function GameCard({ game, onPlay, onToggleFav, onRemove, onRename }: Props) {
  const displayTitle = game.title ?? cleanFilename(game.name);
  // Prefer real box art; fall back to the screen capture if it fails to load.
  const [coverBroken, setCoverBroken] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(displayTitle);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const showCover = game.cover && !coverBroken;

  const commit = () => {
    const next = draft.trim();
    if (next && next !== displayTitle) onRename(game.name, next);
    setEditing(false);
  };

  return (
    <div className="card view" onClick={() => onPlay(game.name)} role="button" tabIndex={0}>
      <button
        className="card__remove"
        title="Remove from library"
        onClick={(e) => {
          e.stopPropagation();
          onRemove(game.name);
        }}
      >
        ×
      </button>
      <button
        className={`card__fav ${game.favorite ? "is-fav" : ""}`}
        title={game.favorite ? "Remove favorite" : "Add favorite"}
        onClick={(e) => {
          e.stopPropagation();
          onToggleFav(game.name, !game.favorite);
        }}
      >
        <IconHeart fill={game.favorite ? "currentColor" : "none"} />
      </button>

      <div className="card__art">
        {showCover ? (
          <img
            className="cover"
            src={game.cover}
            alt={displayTitle}
            loading="lazy"
            onError={() => setCoverBroken(true)}
          />
        ) : game.thumbnail ? (
          <img className="shot" src={game.thumbnail} alt="" />
        ) : (
          <div className="card__placeholder">🎮</div>
        )}
        <div className="card__play">
          <span>▶</span>
        </div>
      </div>

      <div className="card__body">
        {editing ? (
          <input
            ref={inputRef}
            className="card__rename"
            value={draft}
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setDraft(displayTitle);
                setEditing(false);
              }
            }}
          />
        ) : (
          <div className="card__title" title={displayTitle}>
            <span>{displayTitle}</span>
            <button
              className="card__edit"
              title="Rename"
              onClick={(e) => {
                e.stopPropagation();
                setDraft(displayTitle);
                setEditing(true);
              }}
            >
              <IconEdit />
            </button>
          </div>
        )}
        <div className="card__meta">
          <span className="tag">GBA</span>
          {sizeLabel(game.size)}
        </div>
      </div>
    </div>
  );
}
