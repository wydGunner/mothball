# Recovery procedures

Data folder: `%APPDATA%\com.projectvault.desktop\` (Win+R → `%APPDATA%`).

## The app says the database failed its integrity check

Project Vault refuses to open a damaged database and does not modify it.

1. Close Project Vault.
2. Copy the whole data folder somewhere safe.
3. Try SQLite's own recovery (needs `sqlite3.exe` from sqlite.org):
   ```
   sqlite3 vault.db ".recover" | sqlite3 vault-recovered.db
   ```
   Rename `vault.db` → `vault.db.broken`, `vault-recovered.db` → `vault.db`,
   delete `vault.db-wal` / `vault.db-shm`, start the app.
4. Or restore from a backup (below).
5. After recovery: Settings → *Rebuild search index*.

## Restoring a backup

A backup folder (`ProjectVault-backup-<timestamp>`) contains `vault.db`,
`manifest.json` and `export/`.

1. Settings → *Verify a backup* (or check that the folder is intact).
2. Close Project Vault.
3. Rename the current `vault.db` to `vault.db.before-restore`; delete
   `vault.db-wal` and `vault.db-shm`.
4. Copy `vault.db` from the backup into the data folder.
5. Start Project Vault and rescan your projects. Files changed since the backup
   show up as modified; nothing on disk is altered.

Linked project folders are **not** inside backups — only references to them.
If a folder moved, open the project and use *Re-link folder…*.

## Reading a backup without the app

`export/<project>/` is plain text: `project.json`, `notes/*.md`, `files.md`
(all file documentation, with SHA-256 of each file), `tasks.md`,
`timeline.md`. `manifest.json` lists every file with its SHA-256.

## A schema upgrade went wrong

Before upgrading the schema the app writes `backups/pre-migration-v<N>-<time>.db`.
Restore it as above and use the previous app version.

## A drive was unplugged / a project shows “not reachable”

Nothing is marked missing while the root is unreachable. Reconnect the drive
(same letter) and click *Retry*, or use *Re-link folder…* if the path changed.
