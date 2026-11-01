//! Project Vault core library.
//!
//! Everything that touches data lives here so it can be unit-tested without a
//! UI. The desktop shell (src-tauri) and the dev server both talk to it through
//! one entry point: [`api::dispatch`], which takes a command name and JSON
//! arguments and returns JSON.

pub mod api;
pub mod backup;
pub mod db;
pub mod files;
pub mod git;
pub mod json;
pub mod knowledge;
pub mod legacy;
pub mod projects;
pub mod schema;
pub mod search;
pub mod tasks;
pub mod timeline;
pub mod util;

use db::{Db, DbResult};
use std::path::{Path, PathBuf};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Vault {
    pub db: Db,
    pub data_dir: PathBuf,
}

impl Vault {
    /// Opens (or creates) a vault in `data_dir`:
    /// `vault.db`, `managed/` (managed project storage), `backups/`.
    pub fn open(data_dir: &Path) -> DbResult<Vault> {
        std::fs::create_dir_all(data_dir.join("managed")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(data_dir.join("backups")).map_err(|e| e.to_string())?;
        let db_path = data_dir.join("vault.db");
        let existed = db_path.exists();
        let db = Db::open(&db_path)?;
        db.batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        if existed {
            let chk = db.one("PRAGMA quick_check", p![])?.map(|r| r.vals[0].clone());
            if chk.as_ref().and_then(|v| v.as_str()) != Some("ok") {
                return Err(format!(
                    "The vault database failed its integrity check ({:?}). It has not been modified. \
                     Restore from a backup in {} or run the recovery procedure in docs/RECOVERY.md.",
                    chk,
                    data_dir.join("backups").display()
                ));
            }
            let cur = schema::current_version(&db)?;
            if cur < schema::latest_version() && cur > 0 {
                let dest = data_dir.join("backups").join(format!("pre-migration-v{cur}-{}.db", util::now_ms()));
                db.exec("VACUUM INTO ?", p![dest.to_string_lossy().to_string()])?;
            }
        }
        schema::migrate(&db)?;
        let v = Vault { db, data_dir: data_dir.to_path_buf() };
        projects::seed_statuses(&v)?;
        Ok(v)
    }

    pub fn managed_root(&self) -> PathBuf {
        self.data_dir.join("managed")
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("pv-test-{tag}-{}", &util::new_id()[..8]));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub fn vault() -> (TempDir, Vault) {
        let t = TempDir::new("vault");
        let v = Vault::open(&t.0.join("data")).unwrap();
        (t, v)
    }
}
