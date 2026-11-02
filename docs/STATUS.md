# Implementation status

Legend: ✅ implemented and covered by automated tests · 🟡 implemented, partial
or not yet verified on Windows · ⛔ not implemented yet

## How this was verified

* `cargo test` for `vault-core`: 34 unit/integration tests (reconciliation
  cases, imports + undo import, move between projects, remove from vault,
  Smart Save backups + names, dedup, restore, storage location changes and
  unavailable storage, snapshots, retention dry run, notes, tasks, search, git,
  backup/tamper detection, Print Vault import, JSON parser, SHA-256 vectors,
  path traversal, v1→v4 database upgrade).
* `scripts/ui-smoke.mjs`: headless Chromium drives the real UI against the
  real core — 42 steps, no console errors — including Smart Save (cancel
  writes nothing, neutral/descriptive names, backup recorded, conflict
  prompt), version search/compare/restore with keep-current, backups in
  global search with backup-only actions, snapshots (preview, restore, newer
  files kept, automatic undo snapshot), import preview + Undo import (changed
  and unrelated files kept), Remove from Project Vault, keyboard context
  menus, clickable breadcrumbs, the save-mode setting and moving backup
  storage, plus importing the real Print Vault database you supplied.
* **Verified by you on Windows:** the app builds and runs (Nov 1).
* **Still unverified on Windows:** the Monaco diff editor (the tests here use the built-in diff fallback), the Recycle Bin path for Delete / Undo import / Move to another project, the folder picker for backup storage, Explorer “reveal”, the new GitHub panel's sign-in flow, OneDrive online-only file detection, the import picker's type filters, and `tsc` type-checking. Previously unverified: the Tauri shell (`src-tauri/`) and the Windows
  installer have not been compiled, and `tsc` type-checking has not run. The
  build environment had no access to crates.io or the npm registry. The first
  `npm install` + `npm run dev` on Windows (or the CI workflow) is the first
  real build of those parts. The core's linking setup for the desktop build
  (SQLite supplied by the final binary) was verified with a stand-in crate.

## Critical success criteria

| # | Criterion | State |
|---|---|---|
| 1 | Create a project called OMEN v2 | ✅ |
| 2 | Link its existing development directory | ✅ (linked mode, never modifies files) |
| 3 | Browse all project files | ✅ folder tree, filters, sort, search, multi-select |
| 4 | Document what a script does | ✅ purpose, Markdown notes, warnings, tags, custom fields, autosave |
| 5 | Link the script to related scripts, notes, bugs | ✅ link graph + `[[wiki links]]` |
| 6 | Detect modification outside Project Vault | ✅ polling rescans (open, focus, 60 s, F5) |
| 7 | Git history when available | ✅ per-file (follows renames) and project log, diffs, status; plus Project Vault's own version history without Git |
| 8 | Return months later and understand it | ✅ docs follow renames/moves, timeline, revisions |
| 9 | Separate 3D printing project with models, materials, assembly notes | ✅ 3D module fields, print log, STL bounds, Print Vault import |
| 10 | Back up and restore metadata and files reliably | 🟡 verified metadata backups + readable export; restore is a documented manual step; project files are not copied |

## By area

| Area | State | Notes |
|---|---|---|
| Built-in code editor | 🟡 | Monaco (bundled offline, workers included), syntax colors from the app theme, Ctrl+S, conflict detection if the file changes on disk, previous content kept as a version, CRLF/BOM preserved, full-width mode. Tested here with the plain-text stand-in; Monaco itself is first exercised by CI / your build |
| Themes | ✅ | black UI, 10 single-hue or analogous palettes + custom shade, syntax highlighting follows the palette |
| Overview | ✅ | read-only summary (attention items, details, file groups, next tasks, notes, activity) + Edit dialog |
| Dashboard | ✅ | grid/list, filters (status, category, tag, favorites), sorting, favorites, recent, context menu |
| Statuses | ✅ | built-ins + custom |
| Linked / managed projects | ✅ | re-link moved folders, unreachable-drive handling |
| File index & reconciliation | ✅ | stable ids, hash-based moves, review queue for ambiguity |
| Imports | ✅ | file picker, OS drag-and-drop (🟡 desktop-only, untested), dedupe, never overwrite |
| Previews | ✅ | text/code, images; 🟡 no 3D model viewer |
| File docs history | ✅ | doc revisions, detected-change events |
| Duplicates | ✅ | per-file and per-project (by SHA-256) |
| Open with default app / reveal in Explorer | 🟡 | via Tauri opener plugin; untested on Windows |
| Knowledge base | ✅ | tree, drag to nest, categories, Markdown, split view, autosave, revisions, trash |
| Search | ✅ | FTS5 across projects, files, docs, notes, tasks, bugs, journal + substring matching on file names/paths |
| Global search (Ctrl+K) | ✅ | file name + `Project / folder / file` path with clickable project/folder segments, real absolute path on hover, highlights, right-click menu (edit, details, reveal, version history, open, Explorer, copy paths, rename, move to project, remove from Project Vault, delete to Recycle Bin), keyboard (↑↓ Enter Esc Shift+F10); backup copies and snapshots appear as clearly labelled results with backup-only actions (history, compare, restore, save a copy) |
| File rename / delete | ✅ | shared by Files tab and global search; Windows name validation; delete keeps a verified copy, sends to the Recycle Bin (🟡 untested on Windows), and is restorable from Filter → Deleted |
| Tasks & bugs | ✅ | list / Kanban (drag) / calendar, checklists, milestones, bug fields |
| Timeline | ✅ | grouped scan entries, coalesced edits, manual journal |
| Command palette, shortcuts | ✅ | Ctrl K, Ctrl Shift F, Ctrl N, Ctrl B, Alt 1–7, F5, Ctrl S, Alt ← |
| Git integration | ✅ | history, diffs, status; commit, push, pull (fast-forward only), fetch, connect remote, create GitHub repo (GitHub CLI), `git init`. Never resets/rebases/force-pushes |
| File versions (works without Git) | ✅ | automatic for code/text and the project type's key files (3MF/STL/Blend… for 3D printing), side-by-side / unified diff, binary compare (size, STL bounds, SHA-256). Content-addressed store deduplicated by SHA-256 |
| Smart Save | ✅ | Ctrl+S follows Settings → Editor & saving (ask / ask for larger changes / always back up / just save); Ctrl+Shift+S always asks. Description, offline name suggestion (neutral `<file>_vN_<date>` when there's nothing to go on), related task, tags, “back up before saving” vs “keep a copy of what you save” |
| File backups | ✅ | name, description, tags, reason, hash, timestamp; sidecar `.json` per backup; edit details; export a copy; restore dialog shows source → destination and keeps the current file first; verified after writing |
| Version History | ✅ | per-file list with search, “backups only”, compare any two or vs now (Monaco diff 🟡 on Windows, built-in diff here); per-file Timeline with labelled sources (backup / auto version / detected on disk / Project Vault action / Git commit / notes) — only entries with stored contents are restorable |
| Project Backups tab | ✅ | browse every backup in a project by text, date range or quick range; project snapshots (create, details, preview, restore that never deletes newer files and snapshots the current state first, delete) |
| Retention | ✅ | nothing is deleted automatically; “Clean up…” previews the count and asks first; named backups only with an explicit tick |
| Backup storage location | ✅ | Settings: shows path and availability; change copies + verifies, keeps the old folder; clear error when the drive is missing (🟡 folder picker untested on Windows) |
| Project groups | ✅ | your own groups (e.g. “MW2 mods”): dashboard sections (collapsible, drag a card onto a heading, rename/remove group), Filter by → Group, sidebar sections; Recent is listed A→Z so opening a project doesn't reshuffle it; dashboard sorts by name by default and remembers your choice |
| File list grouping | ✅ | Sort ▸ Group by folder / file type / extension, collapsible headers; the Files tab always opens on All files; type chips only show types that exist plus “Other files”, so nothing is hidden (game projects now count .gsc/.csc/.menu, .iwi, .ff/.iwd) |
| Import operations | ✅ | Copy (default), Move (Undo moves files back, never overwriting), or “Edit them where they are” — turns a managed project into a linked one on your folder, copying its existing files there first without overwriting anything |
| Import safety | ✅ | preview shows project + destination folder, copy (never move), per-file conflicts renamed; Undo import from the toast or the Timeline removes only unchanged copies; move files between projects keeps notes and history |
| Project-type file focus | ✅ | Files view groups by role (e.g. Print projects / Meshes / Source models), import picker filters by type |
| Backups | 🟡 | verified, portable; ⛔ scheduled backups, ⛔ optional inclusion of project files, ⛔ in-app restore with preview, ⛔ ZIP export |
| Print Vault import | ✅ | read-only; projects linked in place |
| 3MF metadata extraction | ⛔ | Print Vault's 3MF parser needs ZIP inflate; STL bounds ported |
| Multiple projects open as tabs | ⛔ | one project at a time with tabbed sections |
| Project thumbnails | 🟡 | emoji / category icon; no image thumbnails yet |
| OS file watcher | ⛔ | polling chosen deliberately (no handles held on project folders); a watcher can be added on top |
| Installer | 🟡 | NSIS + MSI config, icons; built by CI, not yet run |
