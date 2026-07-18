// Small helpers to stash binary blobs (save states, battery saves) in
// localStorage, keyed per cartridge.

export function toBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

export function fromBase64(s: string): Uint8Array {
  const bin = atob(s);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

export function store(key: string, bytes: Uint8Array): void {
  localStorage.setItem(key, toBase64(bytes));
}

export function load(key: string): Uint8Array | null {
  const s = localStorage.getItem(key);
  return s ? fromBase64(s) : null;
}
