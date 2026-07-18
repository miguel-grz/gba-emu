import { GbaConsole } from "./GbaConsole";
import { IconClock, IconHeart, IconLibrary, IconSettings } from "./icons";

export type Section = "library" | "favorites" | "recents" | "settings";

interface Props {
  active: Section;
  counts: { library: number; favorites: number };
  onSelect: (s: Section) => void;
}

export function Sidebar({ active, counts, onSelect }: Props) {
  const items: { id: Section; label: string; icon: JSX.Element; count?: number }[] = [
    { id: "library", label: "Library", icon: <IconLibrary />, count: counts.library },
    { id: "favorites", label: "Favorites", icon: <IconHeart />, count: counts.favorites },
    { id: "recents", label: "Recents", icon: <IconClock /> },
    { id: "settings", label: "Settings", icon: <IconSettings /> },
  ];

  return (
    <aside className="sidebar">
      <div className="brand">
        <GbaConsole className="brand__mark" />
        <div className="brand__text">
          <b>Pocket</b>
          <span>GBA Library</span>
        </div>
      </div>

      <nav className="nav">
        {items.map((it) => (
          <button
            key={it.id}
            className={`nav__item ${active === it.id ? "is-active" : ""}`}
            onClick={() => onSelect(it.id)}
          >
            {it.icon}
            <span>{it.label}</span>
            {it.count !== undefined && it.count > 0 && (
              <span className="nav__count">{it.count}</span>
            )}
          </button>
        ))}
      </nav>

      <div className="promo">
        <GbaConsole />
        <b>Game Boy Advance</b>
        <span>The ultimate 32-bit handheld — now in your browser.</span>
      </div>
    </aside>
  );
}
