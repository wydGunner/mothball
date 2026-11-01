import { useState } from 'react';
import { LuFolderOpen, LuShieldCheck, LuDatabaseBackup, LuRefreshCw, LuInfo, LuTriangleAlert } from 'react-icons/lu';
import { call, pickFolder, revealPath, isTauri } from '../ipc';
import { useAction, useLoad } from '../components/common';
import type { Nav } from '../main';

export function SettingsView({ nav }: { nav: Nav }) {
  const info = useLoad(() => call<any>('app.info'), []);
  const [dest, setDest] = useState(() => localStorage.getItem('pv.backupDest') || '');
  const [scope, setScope] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [verifyPath, setVerifyPath] = useState('');
  const [verifyRes, setVerifyRes] = useState<any>(null);
  const act = useAction();

  const backup = async () => {
    setBusy(true);
    localStorage.setItem('pv.backupDest', dest);
    const r = await act(() => call('backup.create', { dest, projectId: scope || null }), 'Backup created and verified');
    setBusy(false);
    if (r) { setResult(r); setVerifyPath((r as any).path); }
  };

  return (
    <div className="page" style={{ maxWidth: 900 }}>
      <h1 style={{ marginBottom: 16 }}>Settings & backup</h1>
      <div className="card pad" style={{ marginBottom: 16 }}>
        <h2 className="row" style={{ marginBottom: 8 }}><LuDatabaseBackup /> Back up</h2>
        <p className="muted small" style={{ marginTop: 0 }}>
          Creates a folder containing a consistent copy of the vault database plus a readable export of every project — notes as Markdown, file documentation, tasks and timeline —
          and a manifest with checksums. The backup is verified right after it is written. It can be read without Project Vault.
        </p>
        <div className="alert warn" style={{ marginBottom: 12 }}><LuTriangleAlert /><div><b>Project files are not copied.</b> Linked projects are backed up as references to their folders plus all metadata. Back up the folders themselves (or keep them in Git) for full recovery. Managed project files live in <span className="mono">{info.data?.dataDir}\managed</span>.</div></div>
        <div className="field"><label>Destination folder (e.g. an external drive)</label>
          <div className="row"><input className="input mono" value={dest} onChange={(e) => setDest(e.target.value)} placeholder="E:\Backups\ProjectVault" /><button className="btn" onClick={async () => { const p = await pickFolder('Backup destination'); if (p) setDest(p); }}><LuFolderOpen /> Browse…</button></div>
        </div>
        <div className="field"><label>Scope</label>
          <select className="input" value={scope} onChange={(e) => setScope(e.target.value)}><option value="">Entire vault</option>{nav.projects.map((p) => <option key={p.id} value={p.id}>Only: {p.name}</option>)}</select>
        </div>
        <button className="btn primary" disabled={!dest || busy} onClick={backup}>{busy ? 'Backing up…' : 'Create backup now'}</button>
        {result && <div className="alert info" style={{ marginTop: 12 }}><LuShieldCheck /><div className="grow">Backup written to <span className="mono">{result.path}</span> — {result.verification.checkedFiles} files verified.</div>
          {isTauri && <button className="btn sm" onClick={() => act(() => revealPath(result.path))}>Show</button>}</div>}
      </div>

      <div className="card pad" style={{ marginBottom: 16 }}>
        <h2 className="row" style={{ marginBottom: 8 }}><LuShieldCheck /> Verify a backup</h2>
        <p className="muted small" style={{ marginTop: 0 }}>Re-checks every file against the manifest checksums and runs an integrity check on the backed-up database.</p>
        <div className="row"><input className="input mono" value={verifyPath} onChange={(e) => setVerifyPath(e.target.value)} placeholder="E:\Backups\ProjectVault\ProjectVault-backup-…" /><button className="btn" onClick={async () => { const p = await pickFolder('Backup folder'); if (p) setVerifyPath(p); }}><LuFolderOpen /></button>
          <button className="btn" disabled={!verifyPath} onClick={async () => setVerifyRes(await act(() => call('backup.verify', { path: verifyPath })))}>Verify</button></div>
        {verifyRes && (verifyRes.ok
          ? <div className="alert info" style={{ marginTop: 10 }}><LuShieldCheck /><div>OK — {verifyRes.checkedFiles} files match their checksums; database integrity check passed.</div></div>
          : <div className="alert bad" style={{ marginTop: 10 }}><LuTriangleAlert /><div>Problems found:<ul>{verifyRes.problems.map((p: string, i: number) => <li key={i}>{p}</li>)}</ul></div></div>)}
        <p className="small dim">Restoring: close Project Vault, then replace <span className="mono">vault.db</span> in the data folder with the one from the backup (keep the old one). See docs/RECOVERY.md. An in-app restore is not implemented yet.</p>
      </div>

      <div className="card pad" style={{ marginBottom: 16 }}>
        <h2 className="row" style={{ marginBottom: 8 }}><LuRefreshCw /> Maintenance</h2>
        <button className="btn" onClick={() => act(() => call('search.rebuild'), 'Search index rebuilt')}>Rebuild search index</button>
      </div>

      <div className="card pad">
        <h2 className="row" style={{ marginBottom: 8 }}><LuInfo /> About</h2>
        <div className="kv">
          <span>Version</span><span>{info.data?.version}</span>
          <span>Database schema</span><span>v{info.data?.schemaVersion} (SQLite {info.data?.sqlite})</span>
          <span>Data folder</span><span className="mono row">{info.data?.dataDir}{isTauri && info.data && <button className="btn ghost sm" onClick={() => act(() => revealPath(info.data.dataDir + '/vault.db'))}>Show</button>}</span>
          <span>Shortcuts</span><span><kbd>Ctrl K</kbd> command palette · <kbd>Ctrl Shift F</kbd> search · <kbd>Ctrl N</kbd> new project · <kbd>Ctrl B</kbd> sidebar · <kbd>Alt 1–6</kbd> project tabs · <kbd>F5</kbd> rescan · <kbd>Ctrl S</kbd> save note · <kbd>Alt ←</kbd> back</span>
        </div>
      </div>
    </div>
  );
}
