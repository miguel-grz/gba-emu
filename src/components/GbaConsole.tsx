// A stylized Game Boy Advance, drawn in SVG — the app's signature mark. Used
// in the sidebar, hero, and empty state. `screen` optionally paints the
// display (a game thumbnail or a glow).

export function GbaConsole({ className, screen }: { className?: string; screen?: string }) {
  return (
    <svg className={className} viewBox="0 0 260 150" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="body" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#7c5cff" />
          <stop offset="1" stopColor="#4c2aa0" />
        </linearGradient>
        <linearGradient id="glow" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#a78bfa" />
          <stop offset="1" stopColor="#5b1a6b" />
        </linearGradient>
      </defs>

      {/* body: a rounded landscape shell with curved grips */}
      <path
        d="M40 20h180c14 0 22 8 24 26 3 22-6 34-6 52 0 16 4 30-14 30-14 0-20-10-34-10H56c-14 0-20 10-34 10C4 128 8 114 8 98c0-18-9-30-6-52C4 28 12 20 26 20z"
        fill="url(#body)"
        stroke="rgba(255,255,255,0.18)"
        strokeWidth="1.5"
      />

      {/* screen bezel + screen */}
      <rect x="78" y="34" width="104" height="72" rx="8" fill="#0c0820" />
      {screen ? (
        <clipPath id="scr">
          <rect x="86" y="42" width="88" height="56" rx="4" />
        </clipPath>
      ) : null}
      {screen ? (
        <image href={screen} x="86" y="42" width="88" height="56" clipPath="url(#scr)" preserveAspectRatio="xMidYMid slice" />
      ) : (
        <rect x="86" y="42" width="88" height="56" rx="4" fill="url(#glow)" opacity="0.85" />
      )}

      {/* d-pad */}
      <g fill="#241a3a">
        <rect x="34" y="66" width="30" height="10" rx="3" />
        <rect x="44" y="56" width="10" height="30" rx="3" />
      </g>

      {/* A / B buttons */}
      <circle cx="210" cy="60" r="9" fill="#ec4899" />
      <circle cx="190" cy="76" r="9" fill="#fbbf24" />

      {/* start / select */}
      <rect x="110" y="112" width="16" height="6" rx="3" fill="#241a3a" transform="rotate(-18 118 115)" />
      <rect x="134" y="112" width="16" height="6" rx="3" fill="#241a3a" transform="rotate(-18 142 115)" />
    </svg>
  );
}
