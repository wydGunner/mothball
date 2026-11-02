# Mothball — architecture

## Layers

```
┌───────────────────────────────────────────────────────────────┐
│ ui/ (React 19 + TypeScript, bundled by esbuild → ui/dist)     │
│   ipc.ts: call(cmd, args) ─┐                                  │
└────────────────────────────┼──────────────────────────────────┘
                             │ Tauri: invoke("vault_call", {cmd, args: JSON})
                             │ Browser/tests: POST /api/<cmd> (vault-devserver)
┌────────────────────────────▼──────────────────────────────────┐
│ src-tauri/ (thin shell)                                        │
│   one command → spawn_blocking → vault_core::api::call         │
│   plugins: dialog (folder/file pickers), opener (open/reveal)  │
│   links libsqlite3-sys "bundled" (SQLite + FTS5 in the .exe)   │
└────────────────────────────┬──────────────────────────────────┘
┌────────────────────────────▼──────────────────────────────────┐
│ crates/vault-core/ (all logic, std-only, unit tested)          │
│   api.rs        command dispatcher (JSON in / JSON out)        │
│   db.rs         safe SQLite FFI wrapper, savepoint transactions│
│   schema.rs     forward-only migrations                        │
│   projects.rs   linked / managed projects, statuses, tags      │
│   files.rs      walk, hash, reconcile, import, file docs, STL  │
│   knowledge.rs  note tree, revisions, link graph               │
│   tasks.rs      tasks, bugs, milestones                        │
│   timeline.rs   grouped activity + manual journal              │
│   search.rs     FTS5 index + safe query building               │
│   git.rs        git CLI: history, diff, commit/push/pull, gh   │
│   versions.rs   stored file versions, compare, restore        │
│   diff.rs       Myers line diff → hunks                        │
│   profiles.rs   file-type groups per project type              │
│   backup.rs     verified metadata backups + Markdown export    │
│   printers.rs   Bambu printer status (MQTT), print logging     │
└───────────────────────────────────────────────────────────────┘
```

The TLS layer for printers lives in `crates/bambu-tls` (rustls, certificate
pinned on first use) so the core stays dependency-free; the dev server and tests
use plain TCP against a fake printer.

Every UI action is a named command. The same `api::call` is used by the
desktop app and the dev server, so the automated UI test (`scripts/ui-smoke.mjs`)
exercises the real backend code.

## Data location

`%APPDATA%\com.projectvault.desktop\` (override with `MOTHBALL_DATA`):

| Path | Content |
|---|---|
| `vault.db` (+ `-wal`, `-shm`) | SQLite database, WAL mode |
| `managed/<slug>-<id8>/` | files of managed projects |
| `objects/ab/cdef…` | stored file versions, named by SHA-256 (each distinct content stored once) |
| `backups/pre-migration-*.db` | automatic copy taken before any schema upgrade |

Linked projects stay where they are. Mothball never renames, moves or
reorganizes their files; it only reads them (and writes into them when you
explicitly import/drop files, never overwriting).

## Database (schema v1)

| Table | Purpose |
|---|---|
| `projects` | name, description, category, status, mode (linked/managed), root path, progress, favorite, `module_json` (category-specific fields), ignore rules |
| `statuses` | built-in (Planned/Active/Paused/Completed/Archived) + custom |
| `tags`, `entity_tags` | tags for projects, files, tasks |
| `files` | one row per **file identity**: random id, current `rel_path`, size, mtime, SHA-256, state (`present`/`missing`), `changed_since_review` |
| `file_docs` | purpose, Markdown notes, warnings, custom fields, `doc_sha256` (file hash the docs were written against) |
| `file_doc_revisions` | earlier documentation versions |
| `file_events` | added / modified / moved / missing / restored / relinked |
| `reconcile_reviews` | uncertain identity matches waiting for the user |
| `notes`, `note_revisions` | hierarchical knowledge base (soft delete + trash) |
| `tasks`, `milestones` | tasks and bug reports (repro/expected/actual/resolution), checklists |
| `links` | undirected relations between files, notes and tasks |
| `timeline` | grouped activity entries and manual journal entries |
| `file_versions` (v2) | stored versions: file, SHA-256 (object name), path, reason (baseline/auto/manual/before-restore), note |
| `search_keys`, `search_index` | FTS5 (`unicode61`, diacritics removed), one row per entity |

IDs are UUID-v4 strings, timestamps Unix milliseconds. Migrations are
append-only; a database newer than the app is refused instead of opened.

## File identity and reconciliation

A scan (manual, on project open, on window focus, every 60 s while visible):

1. Walks the folder read-only (no locks, no symlink following; skips `.git`,
   `node_modules`, OS junk and per-project ignore rules).
2. Unchanged size + mtime → not re-hashed (fast rescans of large trees).
3. Same path, different SHA-256 → **modified**; docs stay, file flagged
   “changed since reviewed”, doc panel warns the notes may be outdated.
4. Path vanished and a new path appeared with the **same hash**, and that hash
   is unique on both sides and among present files → **moved/renamed**; the
   identity (and all docs/links/tags) follows.
5. Anything ambiguous (duplicate copies, same name with different content) →
   no automatic decision; old record becomes **missing**, new file gets a new
   identity, and a review item asks the user. *Documentation never silently
   attaches to the wrong file.*
6. If the project root is unreachable (unplugged drive), the scan aborts and
   nothing is marked missing.

File-system activity is written to the timeline as one summary per scan.

After each scan, `versions::capture` copies the new content of eligible files
(code/text ≤ 2 MB, plus the project type's key files ≤ 150 MB, both
configurable) into the object store, so any two versions can be diffed
(`diff.rs`, Myers line diff) and restored. Restoring first stores the current
content, then writes via temp file + hash check + rename. OneDrive
online-only placeholders are indexed but never read (that would download them).

## Safety measures

* Every relative path from the DB, user input or a backup manifest goes
  through `util::safe_join` (rejects `..`, absolute paths, drive prefixes).
* Imports copy to a temp file, verify SHA-256, then rename; never overwrite
  (`name (2).ext`); duplicate content is skipped.
* Metadata writes are SQLite transactions (savepoints); exports use atomic
  temp-file + rename writes.
* Integrity check on open; pre-migration backup.
* Git is invoked with argv (no shell). Reads use `GIT_OPTIONAL_LOCKS=0`; writes
  happen only from explicit buttons (commit, push, pull --ff-only, remote).
  Remote URLs are validated (https/ssh only).
* Rendered Markdown is sanitized (allow-list) and the webview CSP forbids
  inline scripts.
* Removing a project never deletes a linked folder; completed tasks cannot be
  deleted.

## Why SQLite through a small FFI layer?

The core has no crate dependencies so it can be built and tested anywhere
(including offline CI images). In the desktop build, `libsqlite3-sys` with
`bundled` compiles SQLite (with FTS5) into the executable; the core’s
`extern "C"` declarations resolve against it. Standalone core tests link the
system SQLite (feature `system-sqlite`, default on).
