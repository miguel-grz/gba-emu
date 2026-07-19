// Desktop (Tauri) integration. On the web these functions are inert. Under
// Tauri, the native File > Open ROM menu emits `menu://open-rom`; we open the
// native file dialog, read each chosen ROM's bytes via the `read_rom` command,
// and hand them to the same import pipeline the web UI uses. All @tauri-apps
// imports are dynamic so a pure web build never loads them.

export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * Wire the native "Open ROM" menu to `onOpenRom`. Returns a cleanup function.
 * A no-op (returning a no-op cleanup) when not running under Tauri.
 */
export async function initDesktop(
  onOpenRom: (name: string, bytes: Uint8Array) => void,
): Promise<() => void> {
  if (!isDesktop()) return () => {};

  const { listen } = await import("@tauri-apps/api/event");
  const { invoke } = await import("@tauri-apps/api/core");
  const { open } = await import("@tauri-apps/plugin-dialog");

  const unlisten = await listen("menu://open-rom", async () => {
    const selected = await open({
      multiple: true,
      filters: [{ name: "GBA ROM", extensions: ["gba", "bin"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    for (const path of paths) {
      try {
        const buffer = (await invoke("read_rom", { path })) as ArrayBuffer;
        onOpenRom(baseName(path), new Uint8Array(buffer));
      } catch (e) {
        console.error(`Failed to read ROM: ${path}`, e);
      }
    }
  });

  return unlisten;
}
