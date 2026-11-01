//! Database schema as an ordered list of forward-only migrations.
//!
//! Rules: never edit a shipped migration; append a new one. Each migration
//! runs inside a transaction and is recorded in `schema_migrations`. Before
//! migrating an existing database, the vault copies it to
//! `backups/pre-migration-v{N}.db` (see `Vault::open`).

use crate::db::{Db, DbResult};
use crate::p;

pub const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    r#"
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE statuses (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  color TEXT NOT NULL DEFAULT '#8b93a7',
  sort INTEGER NOT NULL DEFAULT 0,
  builtin INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT 'general',
  status TEXT NOT NULL DEFAULT 'Active',
  mode TEXT NOT NULL CHECK (mode IN ('linked','managed')),
  root_path TEXT NOT NULL,
  icon TEXT NOT NULL DEFAULT '',
  color TEXT NOT NULL DEFAULT '',
  progress INTEGER NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
  favorite INTEGER NOT NULL DEFAULT 0,
  module_json TEXT NOT NULL DEFAULT '{}',
  ignore_globs TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  last_opened_at INTEGER,
  last_scan_at INTEGER,
  root_available INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE tags (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE,
  color TEXT NOT NULL DEFAULT ''
);
CREATE TABLE entity_tags (
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (entity_type, entity_id, tag_id)
);
CREATE INDEX entity_tags_tag ON entity_tags(tag_id);

-- One row per file identity. The id is stable across renames/moves; rel_path
-- is where it was last seen. state: present | missing
CREATE TABLE files (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  rel_path TEXT NOT NULL,
  name TEXT NOT NULL,
  ext TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL DEFAULT 'other',
  size INTEGER NOT NULL DEFAULT 0,
  mtime INTEGER NOT NULL DEFAULT 0,
  sha256 TEXT,
  state TEXT NOT NULL DEFAULT 'present',
  first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL,
  last_changed INTEGER NOT NULL,
  changed_since_review INTEGER NOT NULL DEFAULT 0,
  UNIQUE (project_id, rel_path)
);
CREATE INDEX files_hash ON files(project_id, sha256);
CREATE INDEX files_state ON files(project_id, state);

CREATE TABLE file_docs (
  file_id TEXT PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  purpose TEXT NOT NULL DEFAULT '',
  notes_md TEXT NOT NULL DEFAULT '',
  warnings TEXT NOT NULL DEFAULT '',
  custom_json TEXT NOT NULL DEFAULT '{}',
  doc_sha256 TEXT,         -- content hash the documentation was last written against
  updated_at INTEGER NOT NULL
);
CREATE TABLE file_doc_revisions (
  id TEXT PRIMARY KEY,
  file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  purpose TEXT NOT NULL,
  notes_md TEXT NOT NULL,
  warnings TEXT NOT NULL,
  file_sha256 TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX fdr_file ON file_doc_revisions(file_id, created_at);

CREATE TABLE file_events (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,      -- added | modified | moved | missing | restored | relinked
  detail TEXT NOT NULL DEFAULT '',
  sha256 TEXT,
  at INTEGER NOT NULL
);
CREATE INDEX fe_file ON file_events(file_id, at);

-- Ambiguous identity matches that need the user to decide. Nothing is
-- re-attached automatically when a match is not unique.
CREATE TABLE reconcile_reviews (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  missing_file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  candidate_file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  reason TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (missing_file_id, candidate_file_id)
);

CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  parent_id TEXT REFERENCES notes(id) ON DELETE SET NULL,
  title TEXT NOT NULL,
  body_md TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT 'General',
  sort INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE INDEX notes_project ON notes(project_id, parent_id, sort);
CREATE TABLE note_revisions (
  id TEXT PRIMARY KEY,
  note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  body_md TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX nr_note ON note_revisions(note_id, created_at);

CREATE TABLE milestones (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  due_at INTEGER,
  done INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL DEFAULT 'task' CHECK (kind IN ('task','bug')),
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'todo',
  priority INTEGER NOT NULL DEFAULT 1,
  due_at INTEGER,
  milestone_id TEXT REFERENCES milestones(id) ON DELETE SET NULL,
  checklist_json TEXT NOT NULL DEFAULT '[]',
  repro_steps TEXT NOT NULL DEFAULT '',
  expected TEXT NOT NULL DEFAULT '',
  actual TEXT NOT NULL DEFAULT '',
  resolution TEXT NOT NULL DEFAULT '',
  sort INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  completed_at INTEGER
);
CREATE INDEX tasks_project ON tasks(project_id, status);

-- Generic relation between any two entities (file, note, task) in a project.
CREATE TABLE links (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  a_type TEXT NOT NULL, a_id TEXT NOT NULL,
  b_type TEXT NOT NULL, b_id TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  UNIQUE (a_type, a_id, b_type, b_id)
);
CREATE INDEX links_a ON links(a_type, a_id);
CREATE INDEX links_b ON links(b_type, b_id);

CREATE TABLE timeline (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  summary TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  ref_type TEXT, ref_id TEXT,
  manual INTEGER NOT NULL DEFAULT 0,
  at INTEGER NOT NULL
);
CREATE INDEX timeline_project ON timeline(project_id, at);

-- Maps an entity to its FTS row so updates delete by rowid (FTS5 cannot
-- index UNINDEXED columns; deleting by entity_id would scan the whole index).
CREATE TABLE search_keys (
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  fts_rowid INTEGER NOT NULL,
  PRIMARY KEY (entity_type, entity_id)
);
CREATE INDEX search_keys_rowid ON search_keys(fts_rowid);

CREATE VIRTUAL TABLE search_index USING fts5(
  entity_type UNINDEXED, entity_id UNINDEXED, project_id UNINDEXED,
  title, body, tokenize = 'unicode61 remove_diacritics 2'
);
"#,
)];

pub fn current_version(db: &Db) -> DbResult<i64> {
    db.batch("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)")?;
    db.scalar_i64("SELECT COALESCE(MAX(version),0) FROM schema_migrations", p![])
}

pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.0).unwrap_or(0)
}

pub fn migrate(db: &Db) -> DbResult<i64> {
    let cur = current_version(db)?;
    if cur > latest_version() {
        return Err(format!(
            "This vault was created by a newer Project Vault (schema v{cur}); this build supports up to v{}. Refusing to open it to avoid damage.",
            latest_version()
        ));
    }
    for (v, sql) in MIGRATIONS.iter().filter(|m| m.0 > cur) {
        db.tx(|d| {
            d.batch(sql).map_err(|e| format!("migration v{v} failed: {e}"))?;
            d.exec("INSERT INTO schema_migrations VALUES (?,?)", p![*v, crate::util::now_ms()])?;
            Ok(())
        })?;
    }
    Ok(latest_version())
}
