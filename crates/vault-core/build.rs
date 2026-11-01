// Decide how SQLite gets linked.
//
// VAULT_SQLITE=provided  -> the final binary supplies SQLite (src-tauri sets
//                           this through .cargo/config.toml and depends on
//                           libsqlite3-sys "bundled").
// otherwise              -> link the system SQLite (Linux/macOS dev & CI, or
//                           Windows with VAULT_SQLITE_LIB_DIR pointing at a
//                           directory containing sqlite3.lib).
fn main() {
    println!("cargo:rerun-if-env-changed=VAULT_SQLITE");
    println!("cargo:rerun-if-env-changed=VAULT_SQLITE_LIB_DIR");
    let mode = std::env::var("VAULT_SQLITE").unwrap_or_default();
    if mode == "provided" {
        return;
    }
    if let Ok(dir) = std::env::var("VAULT_SQLITE_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-lib=sqlite3");
}
