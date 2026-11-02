//! Project Vault core library.
//!
//! Everything that touches data lives here so it can be unit-tested without a
//! UI. The desktop shell (src-tauri) and the dev server both talk to it through
//! one entry point: [`api::dispatch`], which takes a command name and JSON
//! arguments and returns JSON.

pub mod api;
pub mod backup;
pub mod db;
pub mod diff;
pub mod files;
pub mod git;
pub mod json;
pub mod knowledge;
pub mod legacy;
pub mod profiles;
pub mod projects;
pub mod schema;
pub mod search;
pub mod snapshots;
pub mod tasks;
pub mod timeline;
pub mod util;
pub mod versions;

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
    #[test]
    fn upgrades_v1_database_with_backup() {
        let t = TempDir::new("mig");
        let data = t.0.join("data");
        std::fs::create_dir_all(&data).unwrap();
        {
            let db = Db::open(&data.join("vault.db")).unwrap();
            crate::schema::current_version(&db).unwrap();
            db.batch(crate::schema::MIGRATIONS[0].1).unwrap();
            db.exec("INSERT INTO schema_migrations VALUES (1, 0)", p![]).unwrap();
            db.exec("INSERT INTO projects(id,name,mode,root_path,created_at,updated_at) VALUES ('x','Old','managed','/tmp',0,0)", p![]).unwrap();
        }
        let v = Vault::open(&data).unwrap();
        assert_eq!(crate::schema::current_version(&v.db).unwrap(), crate::schema::latest_version());
        assert_eq!(v.db.one("SELECT name FROM projects", p![]).unwrap().unwrap().s("name"), "Old");
        let backups: Vec<_> = std::fs::read_dir(data.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(backups.iter().any(|b| b.starts_with("pre-migration-v1")), "{backups:?}");
    }

    pub fn vault() -> (TempDir, Vault) {
        let t = TempDir::new("vault");
        let v = Vault::open(&t.0.join("data")).unwrap();
        (t, v)
    }
}
