// Decide how SQLite gets linked.
//
// feature "system-sqlite" (default): link the platform SQLite (Linux/macOS
//   dev & CI; on Windows set VAULT_SQLITE_LIB_DIR to a folder with sqlite3.lib).
// without it (the desktop app uses `default-features = false`): the final
//   binary supplies SQLite through libsqlite3-sys with the "bundled" feature.
fn main() {
    println!("cargo:rerun-if-env-changed=VAULT_SQLITE_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_SYSTEM_SQLITE").is_none() {
        return;
    }
    if let Ok(dir) = std::env::var("VAULT_SQLITE_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-lib=sqlite3");
}
