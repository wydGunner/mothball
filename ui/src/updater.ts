// Self-update from GitHub Releases (desktop app only).
import { isTauri } from './ipc';

export interface AvailableUpdate { version: string; notes: string; date?: string; install: (onProgress: (pct: number | null) => void) => Promise<void> }

export async function checkForUpdate(): Promise<AvailableUpdate | null> {
  if (!isTauri) return null;
  const { check } = await import('@tauri-apps/plugin-updater');
  const u: any = await check();
  if (!u) return null;
  return {
    version: u.version,
    notes: u.body || '',
    date: u.date,
    install: async (onProgress) => {
      let total = 0, done = 0;
      await u.downloadAndInstall((ev: any) => {
        if (ev.event === 'Started') { total = ev.data.contentLength || 0; onProgress(total ? 0 : null); }
        else if (ev.event === 'Progress') { done += ev.data.chunkLength; onProgress(total ? Math.round((100 * done) / total) : null); }
        else if (ev.event === 'Finished') onProgress(100);
      });
      // On Windows the installer closes the app itself; relaunch covers other cases.
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    },
  };
}
