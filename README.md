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
npm run build      # → src-tauri\target\release\bundle\nsis\Mothball_0.2.0_x64-setup.exe
                   #   src-tauri\target\release\bundle\msi\Mothball_0.2.0_x64_en-US.msi
```

Your data is kept in `%APPDATA%\com.projectvault.desktop\` (the folder keeps its
original name so data from before the rename to Mothball carries over). To use another
folder (portable or test install), set `MOTHBALL_DATA` first.

### Bringing over Print Vault

Dashboard → **Import Print Vault** → choose the folder with
`print_vault.sqlite` (e.g. `D:\PrintVault\3D Prints Vault`). Each project
becomes a linked 3D-printing project pointing at its existing folder.
Print Vault's database is only read. You can keep using it alongside.

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

Copyright (C) 2026 wheresmeep

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE) for the
full text.

In short: you may use, share and change it for free. If you distribute a
modified version, it must stay under the GPL with its full source code.

The app's name and logo are not covered by this license; please don't
distribute modified versions under the same name.
