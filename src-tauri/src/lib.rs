//! Desktop shell. All application logic lives in `vault-core`; this layer
//! only opens the vault in the per-user app-data folder and forwards one IPC
//! command (`vault_call`) to `vault_core::api::call` on a worker thread.

// Links the bundled SQLite (with FTS5) that vault-core's FFI layer uses.
use libsqlite3_sys as _;

use std::sync::{Arc, Mutex};
use tauri::Manager;

struct AppState {
    vault: Arc<Mutex<Option<vault_core::Vault>>>,
    open_error: Option<String>,
}

#[tauri::command]
async fn vault_call(state: tauri::State<'_, AppState>, cmd: String, args: String) -> Result<String, String> {
    if let Some(e) = &state.open_error {
        return Ok(format!(
            "{{\"ok\":false,\"error\":{}}}",
            vault_core::json::Json::from(format!("The vault could not be opened: {e}")).to_string()
        ));
    }
    let vault = state.vault.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let guard = vault.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(v) => vault_core::api::call(v, &cmd, &args),
            None => "{\"ok\":false,\"error\":\"vault not open\"}".to_string(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // MOTHBALL_DATA overrides the location (portable installs, tests);
            // PROJECT_VAULT_DATA is the name from before the rename.
            let dir = match std::env::var_os("MOTHBALL_DATA").or_else(|| std::env::var_os("PROJECT_VAULT_DATA")) {
                Some(d) => std::path::PathBuf::from(d),
                None => app.path().app_data_dir()?,
            };
            let (vault, open_error) = match vault_core::Vault::open(&dir) {
                Ok(v) => (Some(v), None),
                Err(e) => (None, Some(e)),
            };
            let vault = Arc::new(Mutex::new(vault));
            // Background change watching for every project (see vault_core::watch).
            let watched = vault.clone();
            std::thread::spawn(move || {
                let mut w = vault_core::watch::Watcher::default();
                loop {
                    let wait = vault_core::watch::pass(&mut w, |f| {
                        let guard = watched.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(v) = guard.as_ref() {
                            f(v);
                        }
                    });
                    std::thread::sleep(wait);
                }
            });
            // Read-only printer status (see vault_core::printers), over TLS.
            let printers_vault = vault.clone();
            let locker: vault_core::printers::Locker = Arc::new(move |f: &mut dyn FnMut(&vault_core::Vault)| {
                let guard = printers_vault.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(v) = guard.as_ref() {
                    f(v);
                }
            });
            vault_core::printers::start(bambu_tls::connector(), locker);
            app.manage(AppState { vault, open_error });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![vault_call])
        .run(tauri::generate_context!())
        .expect("error while running Mothball");
}
