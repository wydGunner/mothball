<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/mothball-wordmark.png">
    <img src="assets/brand/mothball-wordmark-dark.png" alt="Mothball" width="420">
  </picture>
</p>

# Mothball

*mothball (v.): to store something carefully so it's ready when you need it again.*

A local, offline Windows app for all your projects: find any project, understand
every important file, see what changed, know what to do next.

* **Projects:** link an existing folder (files are never moved or renamed) or
  let the vault manage the files. Statuses, tags, progress, favorites,
  category modules (software, game dev, 3D printing, electronics).
* **File documentation:** purpose, Markdown notes, warnings, custom fields and
  links for every file. Notes follow a file when it's renamed or moved, and
  they never silently attach to the wrong file.
* **Change tracking:** detects edits, moves, deletions and duplicates made
  outside the app; Git history and diffs when the folder is a repository.
* **Knowledge base, tasks & bugs, timeline, universal search, verified backups.**

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md),
[docs/STATUS.md](docs/STATUS.md) (implemented vs. remaining) and
[docs/RECOVERY.md](docs/RECOVERY.md).

## Run it on Windows

Prerequisites (one time):

1. **Rust** — https://rustup.rs (choose the MSVC toolchain; it installs the
   Visual Studio C++ Build Tools if missing).
2. **Node.js 20+** — https://nodejs.org
3. **WebView2** — already on Windows 10/11.
4. **Git for Windows** (optional, for the Git features).

```powershell
cd mothball
npm install
npm run dev        # builds the UI and starts the desktop app
```

Build the installer:

```powershell
npm run build      # → src-tauri\target\release\bundle\nsis\Mothball_<version>_x64-setup.exe
                   #   src-tauri\target\release\bundle\msi\Mothball_<version>_x64_en-US.msi
```

Your data is kept in `%APPDATA%\com.projectvault.desktop\` (the folder keeps its
original name so data from before the rename to Mothball carries over). To use another
folder (portable or test install), set `MOTHBALL_DATA` first.

Projects Mothball manages for you ("Kept in app storage") live under that
folder by default. To keep them on another drive, use Settings → Storage →
*Project folders* → Change…; existing ones are moved (copied, verified, then
removed from the old drive). File backups and snapshots have their own location
in the same place.

## Bambu Lab printer status

Settings → **Printers** → *Add printer*: the printer's IP address, serial number
and LAN access code (all shown on the printer; the serial is also in Bambu
Studio under the device). Mothball then shows what's printing, progress and time
left on the dashboard, and when a print finishes, fails or is stopped it is added
to the print history and timeline of the project whose model or plate file has
the same name. Prints that match no project wait in the dashboard's printer panel
to be assigned (optionally remembered for next time). Loaded filament (AMS slots and
external spool: type, colour, amount left on Bambu spools) shows live, and each
print records what it used — grams measured from Bambu spool levels when Mothball
saw the print start, otherwise the slicer's estimate from the project's .3mf.

It is **read-only**: Mothball never starts, stops or changes anything on the
printer. It connects directly over your network (MQTT over TLS, port 8883), not
through Bambu's cloud. Reading status works on current firmware without
Developer Mode. The printer's certificate is remembered on first connection and
a different one is refused, so another device can't pose as the printer to
collect the access code.

## Stress-testing saving and restoring

`vault-stress` runs thousands of random operations — creating, editing,
renaming, deleting and restoring files; backing up single files, taking and
restoring snapshots, making and restoring full backups, pruning old versions,
reopening the database — against a real folder, and checks after every step
that every file and every saved version has exactly the bytes it should. A
damaged backup must be caught by verification. Crash mode kills it at random
moments and checks the vault still opens, passes SQLite's integrity check, and
that every listed version is intact.

```powershell
cargo run --release --manifest-path crates/vault-stress/Cargo.toml -- --rounds 2000
cargo run --release --manifest-path crates/vault-stress/Cargo.toml -- --dir D:\MothballStress --rounds 2000
cargo run --release --manifest-path crates/vault-stress/Cargo.toml -- --crash 25
```

Add `--long-paths` to include paths over 260 characters, `--seed N` to replay a
run exactly (a failure prints its seed and round). CI runs it on Linux and Windows.

## Development

| Command | What it does |
|---|---|
| `npm run build:ui` / `watch:ui` | bundle the React UI into `ui/dist` |
| `npm run typecheck` | TypeScript check of the UI |
| `npm run test:core` | Rust unit/integration tests (needs a system SQLite — Linux/macOS or WSL; on Windows set `VAULT_SQLITE_LIB_DIR` to a folder with `sqlite3.lib`) |
| `npm run devserver` | run the core + UI in a browser at http://127.0.0.1:1420 (no Tauri) |
| `npm run test:ui` | end-to-end UI test in headless Chromium (build the dev server and UI first) |

CI (`.github/workflows/ci.yml`) runs the core tests, type check and UI test on
Linux, and builds the Windows installers.

### Layout

```
crates/vault-core/   all application logic (Rust, no dependencies) + dev server
src-tauri/           desktop shell: one IPC command, dialogs, opener, packaging
ui/                  React + TypeScript UI (esbuild)
scripts/             end-to-end UI test
docs/                architecture, status, recovery
```

## License

Copyright (C) 2026 wheresmeep / wydGunner

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE) for the
full text.

In short: you may use, share and change it for free. If you distribute a
modified version, it must stay under the GPL with its full source code.

The app's name and logo are not covered by this license; please don't
distribute modified versions under the same name.
