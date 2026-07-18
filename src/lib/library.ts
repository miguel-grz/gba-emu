// Persistent game library backed by IndexedDB. ROMs are far too big for
// localStorage (a Pokémon cartridge is 16 MiB), so the ROM bytes and a small
// metadata record (name, size, thumbnail) live in IndexedDB, keyed by name.

const DB_NAME = "pocket";
const DB_VERSION = 1;
const META = "meta";
const ROMS = "roms";

export interface GameMeta {
  name: string; // storage key (the file name)
  title?: string; // display title (from the game database or a rename)
  cover?: string; // cover-art URL (libretro), if the game is known
  size: number;
  addedAt: number;
  thumbnail?: string; // PNG data URL of a real screen capture
  favorite?: boolean;
  lastPlayed?: number;
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(META)) db.createObjectStore(META, { keyPath: "name" });
      if (!db.objectStoreNames.contains(ROMS)) db.createObjectStore(ROMS);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function promisify<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function listGames(): Promise<GameMeta[]> {
  const db = await openDb();
  const tx = db.transaction(META, "readonly");
  const all = await promisify(tx.objectStore(META).getAll() as IDBRequest<GameMeta[]>);
  return all.sort((a, b) => b.addedAt - a.addedAt);
}

export async function addGame(
  name: string,
  rom: Uint8Array,
  extra: Partial<GameMeta> = {},
): Promise<GameMeta> {
  const db = await openDb();
  const meta: GameMeta = { name, size: rom.byteLength, addedAt: Date.now(), ...extra };
  const tx = db.transaction([META, ROMS], "readwrite");
  tx.objectStore(META).put(meta);
  tx.objectStore(ROMS).put(rom, name);
  await promisify(tx.objectStore(META).get(name));
  return meta;
}

async function updateMeta(name: string, patch: Partial<GameMeta>): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(META, "readwrite");
  const meta = await promisify(tx.objectStore(META).get(name) as IDBRequest<GameMeta>);
  if (meta) tx.objectStore(META).put({ ...meta, ...patch });
}

export function setThumbnail(name: string, thumbnail: string): Promise<void> {
  return updateMeta(name, { thumbnail });
}

export function toggleFavorite(name: string, favorite: boolean): Promise<void> {
  return updateMeta(name, { favorite });
}

export function renameGame(name: string, title: string): Promise<void> {
  return updateMeta(name, { title: title.trim() });
}

export function setDetails(name: string, details: Partial<GameMeta>): Promise<void> {
  return updateMeta(name, details);
}

export function markPlayed(name: string): Promise<void> {
  return updateMeta(name, { lastPlayed: Date.now() });
}

export async function getRom(name: string): Promise<Uint8Array | null> {
  const db = await openDb();
  const tx = db.transaction(ROMS, "readonly");
  const rom = await promisify(tx.objectStore(ROMS).get(name) as IDBRequest<Uint8Array>);
  return rom ?? null;
}

export async function removeGame(name: string): Promise<void> {
  const db = await openDb();
  const tx = db.transaction([META, ROMS], "readwrite");
  tx.objectStore(META).delete(name);
  tx.objectStore(ROMS).delete(name);
  await promisify(tx.objectStore(META).count());
}
