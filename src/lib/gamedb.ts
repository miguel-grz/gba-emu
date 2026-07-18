// Identify a cartridge from its GBA header (a 4-char game code at 0xAC) to
// give it a clean title and fetch cover art. Cover images are NOT bundled —
// they load at runtime from the community libretro thumbnail server (credited
// in Settings), the same source RetroArch uses; if none is found, the card
// falls back to a real screen capture.

const REGION: Record<string, string> = {
  E: "USA",
  P: "Europe",
  J: "Japan",
  S: "Spain",
  F: "France",
  D: "Germany",
  I: "Italy",
};

// Keyed by the first three characters of the game code (which identify the
// game; the fourth is the region). `boxart` is the No-Intro base name the
// libretro server uses.
const GAMES: Record<string, { title: string; boxart?: string }> = {
  BPR: { title: "Pokémon FireRed", boxart: "Pokemon - FireRed Version" },
  BPG: { title: "Pokémon LeafGreen", boxart: "Pokemon - LeafGreen Version" },
  AXV: { title: "Pokémon Ruby", boxart: "Pokemon - Ruby Version" },
  AXP: { title: "Pokémon Sapphire", boxart: "Pokemon - Sapphire Version" },
  BPE: { title: "Pokémon Emerald", boxart: "Pokemon - Emerald Version" },
  BZM: { title: "Zelda: The Minish Cap", boxart: "Legend of Zelda, The - The Minish Cap" },
  AZL: {
    title: "Zelda: A Link to the Past & Four Swords",
    boxart: "Legend of Zelda, The - A Link to the Past & Four Swords",
  },
  AMT: { title: "Metroid Fusion", boxart: "Metroid Fusion" },
  BMX: { title: "Metroid: Zero Mission", boxart: "Metroid - Zero Mission" },
  AMK: { title: "Mario Kart: Super Circuit", boxart: "Mario Kart Super Circuit" },
  AX4: {
    title: "Super Mario Advance 4",
    boxart: "Super Mario Advance 4 - Super Mario Bros. 3",
  },
  AWR: { title: "Advance Wars", boxart: "Advance Wars" },
  AW2: { title: "Advance Wars 2: Black Hole Rising", boxart: "Advance Wars 2 - Black Hole Rising" },
  AGS: { title: "Golden Sun", boxart: "Golden Sun" },
  AGF: { title: "Golden Sun: The Lost Age", boxart: "Golden Sun - The Lost Age" },
  AFE: { title: "Fire Emblem", boxart: "Fire Emblem" },
  BE8: { title: "Fire Emblem: The Sacred Stones", boxart: "Fire Emblem - The Sacred Stones" },
  ASO: { title: "Sonic Advance", boxart: "Sonic Advance" },
  A2N: { title: "Sonic Advance 2", boxart: "Sonic Advance 2" },
  A2C: { title: "Castlevania: Aria of Sorrow", boxart: "Castlevania - Aria of Sorrow" },
  A6B: { title: "Mega Man Battle Network 3: Blue", boxart: "Mega Man Battle Network 3 - Blue" },
};

const THUMB_BASE = "https://thumbnails.libretro.com";
const SYSTEM = "Nintendo - Game Boy Advance";

function gameCode(rom: Uint8Array): string {
  if (rom.length < 0xb0) return "";
  return String.fromCharCode(rom[0xac], rom[0xad], rom[0xae], rom[0xaf]);
}

/** Tidy a filename into a display title (fallback when the game is unknown). */
export function cleanFilename(name: string): string {
  return name
    .replace(/\.(gba|bin)$/i, "")
    .replace(/[([][^)\]]*[)\]]/g, " ") // drop (USA), [!], (V1.1)…
    .replace(/[_.]/g, " ")
    .replace(/\s*-\s*/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

/** Clean display title, preferring the known-game database. */
export function resolveTitle(rom: Uint8Array, filename: string): string {
  const g = GAMES[gameCode(rom).slice(0, 3)];
  return g?.title ?? cleanFilename(filename) ?? filename;
}

/** Cover-art URL from the libretro thumbnail server, or undefined if unknown. */
export function coverUrl(rom: Uint8Array): string | undefined {
  const code = gameCode(rom);
  const g = GAMES[code.slice(0, 3)];
  if (!g?.boxart) return undefined;
  const region = REGION[code[3]] ?? "USA";
  const file = `${g.boxart} (${region}).png`;
  return `${THUMB_BASE}/${encodeURIComponent(SYSTEM)}/Named_Boxarts/${encodeURIComponent(file)}`;
}
