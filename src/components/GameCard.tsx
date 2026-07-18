import { GameMeta } from "../lib/library";

interface Props {
  game: GameMeta;
  onPlay: (name: string) => void;
  onRemove: (name: string) => void;
}

// A stable, cheerful gradient derived from the name, used until a real
// thumbnail has rendered.
function placeholderGradient(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0;
  const h = Math.abs(hash) % 360;
  return `linear-gradient(140deg, hsl(${h} 85% 62%), hsl(${(h + 55) % 360} 80% 55%))`;
}

function prettyName(name: string): string {
  return name.replace(/\.(gba|bin)$/i, "");
}

export function GameCard({ game, onPlay, onRemove }: Props) {
  return (
    <div className="card" onClick={() => onPlay(game.name)} role="button" tabIndex={0}>
      <button
        className="card__remove"
        title="Remove"
        onClick={(e) => {
          e.stopPropagation();
          onRemove(game.name);
        }}
      >
        ×
      </button>
      <div className="card__art">
        {game.thumbnail ? (
          <img src={game.thumbnail} alt="" />
        ) : (
          <div className="card__placeholder" style={{ background: placeholderGradient(game.name) }}>
            🎮
          </div>
        )}
        <div className="card__play">▶</div>
      </div>
      <div className="card__name" title={prettyName(game.name)}>
        {prettyName(game.name)}
      </div>
    </div>
  );
}
