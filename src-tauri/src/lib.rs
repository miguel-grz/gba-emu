// Native desktop shell for Pocket. The emulator itself runs as WebAssembly in
// the webview; this crate only provides the native window, the native menu, and
// a command to read a ROM file the user picks via the native dialog.

use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::Emitter;

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
