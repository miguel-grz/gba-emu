// Persistent game library backed by IndexedDB. ROMs are far too big for
// localStorage (a Pokémon cartridge is 16 MiB), so the ROM bytes and a small
// metadata record (name, size, thumbnail) live in IndexedDB, keyed by name.

const DB_NAME = "pocket";
const DB_VERSION = 1;
const META = "meta";
const ROMS = "roms";

export interface GameMeta {
  name: string;
  size: number;
  addedAt: number;
  thumbnail?: string; // PNG data URL
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

export async function addGame(name: string, rom: Uint8Array): Promise<GameMeta> {
  const db = await openDb();
  const meta: GameMeta = { name, size: rom.byteLength, addedAt: Date.now() };
  const tx = db.transaction([META, ROMS], "readwrite");
  tx.objectStore(META).put(meta);
  tx.objectStore(ROMS).put(rom, name);
  await promisify(tx.objectStore(META).get(name));
  return meta;
}

export async function setThumbnail(name: string, thumbnail: string): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(META, "readwrite");
  const meta = await promisify(tx.objectStore(META).get(name) as IDBRequest<GameMeta>);
  if (meta) {
    meta.thumbnail = thumbnail;
    tx.objectStore(META).put(meta);
  }
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
