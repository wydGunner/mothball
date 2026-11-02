# Releasing & automatic updates

Installed copies of Project Vault check
`https://github.com/wydGunner/project-vault/releases/latest/download/latest.json`
on startup (and every 6 hours) and offer **Update to vX.Y.Z** in the top bar.
Updates are signed; the app only installs files signed with the project's key.

## One-time setup

1. Generate the update signing key (on your PC, in the project folder):
   ```
   npx tauri signer generate -w "%USERPROFILE%\.tauri\project-vault.key"
   ```
   Choose a password (or leave it empty). Keep the `.key` file safe and
   private — if it is lost, existing installs can't verify new updates and
   must be reinstalled manually once.
2. Put the **public** key (`project-vault.key.pub`) into
   `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`.
3. On GitHub: repository → Settings → Secrets and variables → Actions →
   New repository secret:
   * `TAURI_SIGNING_PRIVATE_KEY` = full contents of `project-vault.key`
   * `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = the password (create it even if empty)

The repository must be **public** for installed apps to download updates
without credentials.

## Each release

```
npm run release -- 0.3.0
git push --follow-tags
```

The `Release` workflow builds the signed NSIS installer and publishes the
GitHub Release with `latest.json`. Installed apps pick it up automatically.

Local `npm run build` now also produces updater signatures, so it needs
`TAURI_SIGNING_PRIVATE_KEY` set; `npm run dev` does not.
