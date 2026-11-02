# Implementation status

Legend: ✅ implemented and covered by automated tests · 🟡 implemented, partial
or not yet verified on Windows · ⛔ not implemented yet

## How this was verified

* `cargo test` for `vault-core`: 25 unit/integration tests (incl. line diff, versions/restore, Git commit/push to a local remote, v1→v2 database upgrade) (reconciliation
  cases, imports, notes, tasks, search, git, backup/tamper detection, Print
  Vault import, JSON parser, SHA-256 test vectors, path traversal).
* `scripts/ui-smoke.mjs`: headless Chromium drives the real UI against the
  real core through all 10 critical success criteria plus versions, Git commit/push and the type filters (23 steps, no console
  errors), including importing the real Print Vault database you supplied.
* **Verified by you on Windows:** the app builds and runs (Nov 1).
* **Still unverified on Windows:** the new GitHub panel's sign-in flow, OneDrive online-only file detection, the import picker's type filters, and `tsc` type-checking. Previously unverified: the Tauri shell (`src-tauri/`) and the Windows
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
| Search | ✅ | FTS5 across projects, files, docs, notes, tasks, bugs, journal |
| Tasks & bugs | ✅ | list / Kanban (drag) / calendar, checklists, milestones, bug fields |
| Timeline | ✅ | grouped scan entries, coalesced edits, manual journal |
| Command palette, shortcuts | ✅ | Ctrl K, Ctrl Shift F, Ctrl N, Ctrl B, Alt 1–6, F5, Ctrl S, Alt ← |
| Git integration | ✅ | history, diffs, status; commit, push, pull (fast-forward only), fetch, connect remote, create GitHub repo (GitHub CLI), `git init`. Never resets/rebases/force-pushes |
| File versions (works without Git) | ✅ | automatic for code/text and the project type's key files (3MF/STL/Blend… for 3D printing), manual “save version”, side-by-side / unified diff, binary compare (size, STL bounds), restore with automatic undo point, pruning. Content-addressed store deduplicated by SHA-256 |
| Project-type file focus | ✅ | Files view groups by role (e.g. Print projects / Meshes / Source models), import picker filters by type |
| Backups | 🟡 | verified, portable; ⛔ scheduled backups, ⛔ optional inclusion of project files, ⛔ in-app restore with preview, ⛔ ZIP export |
| Print Vault import | ✅ | read-only; projects linked in place |
| 3MF metadata extraction | ⛔ | Print Vault's 3MF parser needs ZIP inflate; STL bounds ported |
| Multiple projects open as tabs | ⛔ | one project at a time with tabbed sections |
| Project thumbnails | 🟡 | emoji / category icon; no image thumbnails yet |
| OS file watcher | ⛔ | polling chosen deliberately (no handles held on project folders); a watcher can be added on top |
| Installer | 🟡 | NSIS + MSI config, icons; built by CI, not yet run |
