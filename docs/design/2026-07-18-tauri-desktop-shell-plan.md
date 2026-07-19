# Tauri Desktop Shell Implementation Plan

> Task-by-task implementation plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Package the existing Pocket WASM web frontend as a native Tauri v2 desktop app with a native menu whose File → Open ROM… feeds the existing import flow via a native file dialog.

**Architecture:** Tauri v2 is a pure shell around the current Vite/React frontend; the emulator keeps running as WebAssembly in the webview. A native menu item emits an event; the frontend opens a native dialog, reads the chosen file's bytes through a tiny Rust command, and calls the existing import pipeline.

**Tech Stack:** Tauri v2 (Rust), React + TypeScript + Vite, `@tauri-apps/api`, `@tauri-apps/plugin-dialog`.

## Deviations from the spec (planning-time refinements)

- **ROM reading uses a small Rust command returning raw bytes (`tauri::ipc::Response`), not `tauri-plugin-fs`.** The dialog (JS `@tauri-apps/plugin-dialog`) returns a path; a custom `read_rom(path)` command reads it with `std::fs` and returns raw bytes (received as an ArrayBuffer in JS). This is more efficient for large ROMs (up to 16 MiB) than JSON-serializing through the fs plugin, and needs no filesystem permission scope (app-defined commands don't require capability entries). `tauri-plugin-fs` is therefore dropped from the design.
- **Dev port is 1420** (Tauri's convention), pinned via `vite.config.ts` `server.port` + `strictPort`, so `devUrl` is stable. (The spec left the exact port open.)

## Global Constraints

- **Tauri v2** only. Product name `Pocket`, identifier `dev.pocket.gba`, version `0.1.0`.
- **Dev URL / port:** `http://localhost:1420`, pinned with `strictPort`.
- **No changes to `gba-core` (`core/`) or `web/` crate logic.** The Rust core is untouched; no new `cargo test` there.
- **`npx tsc --noEmit` (repo root) must pass** after the frontend changes.
- **The web path must not regress:** drag-drop / file-input import still works in the browser preview after the `importRom` refactor.
- **App-defined Rust commands** (e.g. `read_rom`) do not need capability permissions; only plugin/core commands do.
- **First `cargo build` / `tauri` build is slow** (compiles WebKit-binding crates). Use a long Bash timeout (600000 ms) for Tauri Rust builds.
- **Commits must NOT include a `Co-Authored-By: Claude` trailer.**
- **External Tauri v2 API note:** the exact menu-builder method signatures can vary across tauri 2.x point releases. Where the compiler rejects a signature, adjust to the installed version (consult `cargo doc -p tauri --open` or the error) — keep the behavior identical. `cargo build` is the gate.
- This environment can drive the localhost webview preview but **cannot screenshot or click a native macOS window**; native-window visual confirmation is the developer's, not a claimed pass.

---

## File structure

- `vite.config.ts` — **modify.** Pin `server.port: 1420`, `strictPort: true`.
- `package.json` — **modify.** Add `@tauri-apps/cli` (dev), `@tauri-apps/api` + `@tauri-apps/plugin-dialog` (runtime), and a `"tauri": "tauri"` script.
- `Cargo.toml` (root) — **modify.** Add `"src-tauri"` to workspace `members`.
- `src-tauri/` — **create** (via `tauri init`, then customized):
  - `Cargo.toml` — deps `tauri` v2, `tauri-plugin-dialog` v2, `serde`, `serde_json`; build-dep `tauri-build` v2.
  - `tauri.conf.json` — app/window/bundle/build hooks.
  - `build.rs`, `icons/` — generated, left as-is.
  - `capabilities/default.json` — `core:default` + `dialog:default`.
  - `src/main.rs` — generated entrypoint calling `pocket_lib::run()`.
  - `src/lib.rs` — **replace.** Native menu + `on_menu_event` emit + `read_rom` command + dialog plugin.
- `src/lib/desktop.ts` — **create.** Tauri detection + menu-event listener that opens the dialog, reads bytes, calls back.
- `src/App.tsx` — **modify.** Extract `importRom`; wire `initDesktop`.

---

## Task 1: Scaffold the Tauri shell and wire it into the project

Produces a Tauri app that opens the existing web frontend in a native window (default menu, no custom items yet), building from the workspace.

**Files:**
- Create: `src-tauri/**` (via `tauri init`)
- Modify: `vite.config.ts`, `package.json`, `Cargo.toml` (root), `src-tauri/tauri.conf.json`

**Interfaces:**
- Produces: an `npm run tauri dev`/`build`-capable project; `src-tauri` crate `pocket` with lib `pocket_lib` exposing `run()` (used by Task 2).

- [ ] **Step 1: Install the Tauri CLI and pin the Vite dev port**

Run:
```bash
cd /Users/miguelangel/Documents/gba-emu
npm install -D @tauri-apps/cli@^2
```

Edit `vite.config.ts` to pin the port (Tauri needs a fixed `devUrl`). Replace the `server` block so the file reads:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The WASM package under web/pkg lives outside the frontend root; allow Vite's
// dev server to serve it. The port is pinned so Tauri's devUrl stays stable.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [".."] },
  },
});
```

- [ ] **Step 2: Scaffold `src-tauri` with `tauri init`**

Run (non-interactive):
```bash
cd /Users/miguelangel/Documents/gba-emu
npx tauri init --ci \
  --app-name Pocket \
  --window-title Pocket \
  --frontend-dist ../dist \
  --dev-url http://localhost:1420 \
  --before-dev-command "npm run dev" \
  --before-build-command "npm run wasm && npm run build"
```
Expected: creates `src-tauri/` with `Cargo.toml`, `tauri.conf.json`, `build.rs`, `src/main.rs`, `src/lib.rs`, `capabilities/default.json`, and `icons/`.

- [ ] **Step 3: Add `src-tauri` to the Cargo workspace**

Edit the root `Cargo.toml` — change the members line to include `src-tauri`:

```toml
[workspace]
members = ["core", "web", "src-tauri"]
resolver = "2"
```

If `src-tauri/Cargo.toml` contains a `[profile.*]` table, delete it (profiles are only allowed in the workspace root, which already defines `[profile.release]`). If it contains its own `[workspace]` table, delete that too.

- [ ] **Step 4: Configure `tauri.conf.json` (identifier, window, bundle)**

Edit `src-tauri/tauri.conf.json` so these fields are set (leave other generated fields intact). The identifier MUST change from the generated `com.tauri.dev` placeholder or the build fails:

```json
{
  "productName": "Pocket",
  "version": "0.1.0",
  "identifier": "dev.pocket.gba",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run wasm && npm run build",
    "frontendDist": "../dist"
  },
  "app": {
    "withGlobalTauri": false,
    "windows": [
      {
        "title": "Pocket",
        "width": 1200,
        "height": 820,
        "minWidth": 940,
        "minHeight": 640,
        "resizable": true
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "dmg",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```
(Match the `icon` array to the files `tauri init` actually generated under `src-tauri/icons/`. Keep the generated `$schema` field if present.)

- [ ] **Step 5: Add the `tauri` npm script**

Edit `package.json` `scripts`, adding:
```json
"tauri": "tauri"
```

- [ ] **Step 6: Verify the frontend build is unaffected**

Run:
```bash
cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit
```
Expected: no output.

- [ ] **Step 7: Verify the Tauri crate compiles**

Run (allow up to 10 minutes for the first build):
```bash
cd /Users/miguelangel/Documents/gba-emu && cargo build -p pocket
```
Expected: compiles successfully (warnings from the generated template are acceptable). If the crate name differs from `pocket`, use the `name` under `[package]` in `src-tauri/Cargo.toml`.

- [ ] **Step 8: Commit**

```bash
cd /Users/miguelangel/Documents/gba-emu
git add -A
git commit -m "Scaffold Tauri v2 desktop shell around the web frontend"
```

---

## Task 2: Native menu, Open-ROM event, and the read_rom command

Replaces the generated `src/lib.rs` with a native menu (File → Open ROM…), an event emitted on click, the `read_rom` command that returns raw file bytes, and registration of the dialog plugin.

**Files:**
- Modify: `src-tauri/src/lib.rs` (replace body)
- Modify: `src-tauri/Cargo.toml` (add `tauri-plugin-dialog`)
- Modify: `src-tauri/capabilities/default.json` (add `dialog:default`)

**Interfaces:**
- Produces (consumed by Task 3): the webview receives a `menu://open-rom` event with no payload; a command `read_rom(path: string) -> ArrayBuffer` reads a file's bytes.

- [ ] **Step 1: Add the dialog plugin crate**

Run:
```bash
cd /Users/miguelangel/Documents/gba-emu/src-tauri && cargo add tauri-plugin-dialog@2
```

- [ ] **Step 2: Replace `src-tauri/src/lib.rs`**

Write `src-tauri/src/lib.rs`:

```rust
// Native desktop shell for Pocket. The emulator itself runs as WebAssembly in
// the webview; this crate only provides the native window, the native menu, and
// a command to read a ROM file the user picks via the native dialog.

use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager};

// Read a ROM file chosen through the native dialog and return its raw bytes.
// Returning `ipc::Response` sends the bytes as an ArrayBuffer to JS (efficient
// for multi-MiB ROMs) rather than a JSON number array.
#[tauri::command]
fn read_rom(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(&path)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![read_rom])
        .menu(|handle| {
            let open_rom = MenuItemBuilder::new("Open ROM…")
                .id("open-rom")
                .accelerator("CmdOrCtrl+O")
                .build(handle)?;

            let file = SubmenuBuilder::new(handle, "File")
                .item(&open_rom)
                .separator()
                .quit()
                .build()?;

            // A minimal app + edit menu so standard macOS shortcuts feel native.
            let app_menu = SubmenuBuilder::new(handle, "Pocket")
                .about(None)
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;

            let edit = SubmenuBuilder::new(handle, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;

            MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&file)
                .item(&edit)
                .build()
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "open-rom" {
                let _ = app.emit("menu://open-rom", ());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Pocket");
}
```

Note (external API): if the compiler rejects a builder method (e.g. `.about(None)` arity, or `event.id().as_ref()`), adjust to the installed `tauri` 2.x signature — the behavior must stay identical (a "File" submenu with an "open-rom" item that emits `menu://open-rom`, plus quit/edit). Leave `src/main.rs` as generated (it already calls `pocket_lib::run()`).

- [ ] **Step 3: Grant the dialog permission**

Edit `src-tauri/capabilities/default.json` so its `permissions` array includes `"dialog:default"` alongside the generated `core:default` (do not remove existing entries). Example result:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": ["core:default", "dialog:default"]
}
```
(Keep whatever `$schema`/other fields `tauri init` generated; only ensure `dialog:default` is added to `permissions`. The window label must match the one in `tauri.conf.json` — the generated default is `main`.)

- [ ] **Step 4: Verify the Tauri crate compiles**

Run (allow up to 10 minutes):
```bash
cd /Users/miguelangel/Documents/gba-emu && cargo build -p pocket
```
Expected: compiles successfully.

- [ ] **Step 5: Commit**

```bash
cd /Users/miguelangel/Documents/gba-emu
git add -A
git commit -m "Add native menu, Open-ROM event, and read_rom command"
```

---

## Task 3: Frontend bridge — importRom refactor and desktop listener

Extracts the reusable `importRom` from `addRom`, adds `desktop.ts` that (only under Tauri) listens for the menu event and drives the native dialog + `read_rom`, and wires it in `App.tsx`.

**Files:**
- Create: `src/lib/desktop.ts`
- Modify: `src/App.tsx`
- Modify: `package.json` (add `@tauri-apps/api`, `@tauri-apps/plugin-dialog`)

**Interfaces:**
- Consumes: the `menu://open-rom` event and `read_rom` command from Task 2.
- Produces: no new exports consumed by later tasks (final task).

- [ ] **Step 1: Install the JS Tauri packages**

Run:
```bash
cd /Users/miguelangel/Documents/gba-emu
npm install @tauri-apps/api@^2 @tauri-apps/plugin-dialog@^2
```

- [ ] **Step 2: Create `src/lib/desktop.ts`**

Write `src/lib/desktop.ts`:

```ts
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
      const buffer = (await invoke("read_rom", { path })) as ArrayBuffer;
      onOpenRom(baseName(path), new Uint8Array(buffer));
    }
  });

  return unlisten;
}
```

- [ ] **Step 3: Refactor `App.tsx` to extract `importRom` and wire the desktop listener**

In `src/App.tsx`:

Add the import near the other `./lib/*` imports:
```ts
import { initDesktop } from "./lib/desktop";
```

Replace the existing `addRom` callback:
```ts
  const addRom = useCallback(
    async (file: File) => {
      setError(null);
      const bytes = new Uint8Array(await file.arrayBuffer());
      setBusy(file.name);
      try {
        await addGame(file.name, bytes, {
          title: resolveTitle(bytes, file.name),
          cover: coverUrl(bytes),
        });
        await refreshGames();
        const thumb = await generateThumbnail(bytes);
        await setThumbnail(file.name, thumb);
        await refreshGames();
      } catch (e) {
        setError(String(e));
      }
      setBusy(null);
    },
    [refreshGames],
  );
```
with an extracted `importRom` plus a thin `addRom` adapter:
```ts
  const importRom = useCallback(
    async (name: string, bytes: Uint8Array) => {
      setError(null);
      setBusy(name);
      try {
        await addGame(name, bytes, {
          title: resolveTitle(bytes, name),
          cover: coverUrl(bytes),
        });
        await refreshGames();
        const thumb = await generateThumbnail(bytes);
        await setThumbnail(name, thumb);
        await refreshGames();
      } catch (e) {
        setError(String(e));
      }
      setBusy(null);
    },
    [refreshGames],
  );

  const addRom = useCallback(
    async (file: File) => {
      importRom(file.name, new Uint8Array(await file.arrayBuffer()));
    },
    [importRom],
  );
```

Add an effect (near the other effects) that wires the native menu:
```ts
  // Native desktop menu (File > Open ROM). No-op on the web.
  useEffect(() => {
    let cleanup = () => {};
    initDesktop(importRom).then((fn) => {
      cleanup = fn;
    });
    return () => cleanup();
  }, [importRom]);
```

- [ ] **Step 4: Verify types**

Run:
```bash
cd /Users/miguelangel/Documents/gba-emu && npx tsc --noEmit
```
Expected: no output.

- [ ] **Step 5: Verify the web import path did not regress**

Ensure the browser preview is running (`preview_start` name `gba-emu`), open the tab, and import a `.gba` via the file input or drag-drop; confirm the game appears in the library with a title/thumbnail exactly as before. Check `read_console_messages` (onlyErrors) shows no new errors. (This exercises the `importRom` refactor on the web path; `isDesktop()` is false here, so `initDesktop` is a no-op.)

- [ ] **Step 6: Commit**

```bash
cd /Users/miguelangel/Documents/gba-emu
git add -A
git commit -m "Route ROM import through importRom; wire native Open-ROM menu"
```

---

## Final verification (controller)

Not a task — done after Task 3 review:

- `npm run tauri dev` launched in the background; confirm from logs that Vite starts on :1420 and Tauri creates the window with no Rust panics or errors. The interactive native-window check (File → Open ROM… opens a picker, a chosen game runs) is the developer's, since this environment cannot drive a native macOS window — state this explicitly rather than claim it.
- Optionally, `npm run tauri build` to confirm the bundle produces a `.app`/`.dmg` (slow; note if skipped).

---

## Self-review notes

- **Spec coverage:** project structure + workspace + config → Task 1; native menu + Open-ROM bridge (menu, event, byte reading) → Task 2 (Rust) + Task 3 (JS); `importRom` refactor + listener wiring → Task 3; dev/build wiring → Task 1 config + Task 3 deps; verification limits → Final verification. The spec's `plugin-fs` is intentionally replaced by the `read_rom` command (documented under Deviations).
- **Out-of-scope items** (Windows/Linux packaging, native FS storage, auto-update/signing, tray) are not implemented — matches the spec.
- **Type/name consistency:** `importRom(name, bytes)` signature matches between `App.tsx` and `desktop.ts`'s `onOpenRom` callback; `read_rom`'s Rust command name matches the JS `invoke("read_rom", { path })`; the `menu://open-rom` event string matches between `lib.rs` emit and `desktop.ts` listen; crate/lib names `pocket`/`pocket_lib` match between `main.rs`, `Cargo.toml`, and the `cargo build -p pocket` commands.
