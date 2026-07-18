import { GameMeta } from "../lib/library";
import { IconHeart } from "./icons";

interface Props {
  game: GameMeta;
  onPlay: (name: string) => void;
  onToggleFav: (name: string, favorite: boolean) => void;
  onRemove: (name: string) => void;
}

function prettyName(name: string): string {
  return name.replace(/\.(gba|bin)$/i, "");
}

function sizeLabel(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.round(bytes / 1024)} KB`;
}

export function GameCard({ game, onPlay, onToggleFav, onRemove }: Props) {
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
        {game.thumbnail ? (
          <img src={game.thumbnail} alt="" />
        ) : (
          <div className="card__placeholder">🎮</div>
        )}
        <div className="card__play">
          <span>▶</span>
        </div>
      </div>

      <div className="card__body">
        <div className="card__title" title={prettyName(game.name)}>
          {prettyName(game.name)}
        </div>
        <div className="card__meta">
          <span className="tag">GBA</span>
          {sizeLabel(game.size)}
        </div>
      </div>
    </div>
  );
}
