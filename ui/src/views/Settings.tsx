import { useState } from 'react';
import { LuFolderOpen, LuShieldCheck, LuDatabaseBackup, LuRefreshCw, LuInfo, LuTriangleAlert } from 'react-icons/lu';
import { call, pickFolder, revealPath, isTauri } from '../ipc';
import { useAction, useConfirm, useLoad, useToast } from '../components/common';
import { getSaveMode, setSaveMode, getLargeThreshold, setLargeThreshold, type SaveMode } from './SmartSave';
import { bytes } from '../format';
import type { Nav } from '../main';
import { THEMES, applyTheme, getTheme, themeFromColor } from '../theme';

function Appearance() {
  const [cur, setCur] = useState(getTheme());
  const pick = (t: typeof cur) => { applyTheme(t); setCur(t); };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2 style={{ marginBottom: 4 }}>Appearance</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Black background with one color family: either shades of a single color, or neighbouring colors that go together.</p>
      <div className="row wrap" style={{ gap: 10 }}>
        {THEMES.map((t) => (
          <button key={t.id} className={'theme-card' + (cur.id === t.id ? ' on' : '')} onClick={() => pick(t)} aria-label={t.name}>
            <div className="bars"><span style={{ background: t.a1 }} /><span style={{ background: t.a2 }} /><span style={{ background: t.a3 }} /></div>
            <small>{t.name}</small>
          </button>
        ))}
        <label className={'theme-card' + (cur.id === 'custom' ? ' on' : '')} title="Pick any color; shades are generated from it">
          <div className="bars" style={{ position: 'relative' }}>
            {cur.id === 'custom' ? <><span style={{ background: cur.a1 }} /><span style={{ background: cur.a2 }} /><span style={{ background: cur.a3 }} /></> : <span style={{ background: 'conic-gradient(#f43f5e, #ffb224, #a3e635, #22d3ee, #8b5cf6, #f43f5e)' }} />}
            <input type="color" value={cur.a1} onChange={(e) => pick(themeFromColor(e.target.value))} style={{ position: 'absolute', inset: 0, opacity: 0, cursor: 'pointer' }} />
          </div>
          <small>Custom shade</small>
        </label>
      </div>
      <div className="row" style={{ marginTop: 14, gap: 10 }}>
        <button className="btn primary sm">Primary</button>
        <span className="pill"><span className="dot on" />Active</span>
        <span className="badge warn">changed</span>
        <span className="prio p3">urgent</span>
        <div className="progress" style={{ width: 120 }}><div style={{ width: '60%' }} /></div>
      </div>
    </div>
  );
}

const SAVE_MODES: { id: SaveMode; label: string; hint: string }[] = [
  { id: 'ask', label: 'Ask every time', hint: 'Ctrl+S opens the Smart Save dialog (name, description, backup).' },
  { id: 'ask-large', label: 'Ask only for larger changes', hint: 'Small edits save straight away; the dialog appears when many lines changed.' },
  { id: 'auto', label: 'Always back up before saving, without asking', hint: 'Ctrl+S saves immediately and keeps a copy of the previous state.' },
  { id: 'manual', label: 'Just save', hint: 'Ctrl+S saves immediately. Use Ctrl+Shift+S or “Save with backup…” when you want a backup.' },
];

function EditorSaving() {
  const [mode, setMode] = useState<SaveMode>(getSaveMode);
  const [lines, setLines] = useState(getLargeThreshold);
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2 style={{ marginBottom: 4 }}>Editor &amp; saving</h2>
      <p className="muted small" style={{ marginTop: 0 }}>What happens when you press Ctrl+S in Project Vault’s editor. Ctrl+Shift+S always opens the Smart Save dialog.</p>
      {SAVE_MODES.map((m) => (
        <label key={m.id} className="row" style={{ alignItems: 'flex-start', marginBottom: 6 }}>
          <input type="radio" name="savemode" checked={mode === m.id} onChange={() => { setMode(m.id); setSaveMode(m.id); }} style={{ marginTop: 3 }} />
          <span><b>{m.label}</b><div className="small dim">{m.hint}</div></span>
        </label>
      ))}
      {mode === 'ask-large' && <div className="row small" style={{ marginLeft: 24 }}>“Larger” means at least <input className="input" type="number" min={1} style={{ width: 70, padding: '2px 6px' }} value={lines} onChange={(e) => { const n = Math.max(1, Number(e.target.value) || 1); setLines(n); setLargeThreshold(n); }} /> changed lines.</div>}
      <p className="small dim" style={{ marginBottom: 0 }}>Separately, each project can keep automatic versions whenever a file changes on disk (project ⋯ → Edit → Files &amp; versions).</p>
    </div>
  );
}

function BackupStorage() {
  const st = useLoad(() => call<any>('storage.get'), []);
  const [busy, setBusy] = useState(false);
  const confirm = useConfirm();
  const act = useAction();
  const toast = useToast();
  const change = async () => {
    const dir = await pickFolder('Choose where Project Vault keeps file backups and snapshots');
    if (!dir) return;
    if (!(await confirm({ title: 'Move backup storage?', confirm: 'Copy and switch', body: <>All stored backups and snapshots are <b>copied</b> to <span className="mono">{dir}</span> and verified. Project Vault then uses the new location. The old folder is left as it is — delete it yourself once you’re happy.</> }))) return;
    setBusy(true);
    const r: any = await act(() => call('storage.set', { path: dir }));
    setBusy(false);
    if (r) { toast(`Backup storage is now ${r.root} (${r.copied} object(s) copied)`, 'success'); st.reload(); }
  };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2 style={{ marginBottom: 4 }}>Backup storage location</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Where file backups, automatic versions and project snapshots are stored. Each backup also gets a readable <code>.json</code> description next to it, so the folder makes sense even without Project Vault.</p>
      {st.error ? <div className="alert bad">{st.error}</div> : !st.data ? <div className="muted small">Loading…</div> : <>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <span className="mono small grow" style={{ wordBreak: 'break-all' }}>{st.data.root}</span>
          {st.data.available ? <span className="badge ok">available</span> : <span className="badge bad">not reachable</span>}
          {isTauri && st.data.available && <button className="btn sm icon" title="Show in Explorer" onClick={() => act(() => revealPath(st.data.root))}><LuFolderOpen /></button>}
          <button className="btn sm" disabled={busy} onClick={change}>{busy ? 'Copying…' : 'Change…'}</button>
        </div>
        {!st.data.available && <div className="alert bad small" style={{ marginTop: 8 }}>The folder can’t be reached (unplugged drive?). Backups can’t be saved or restored until it’s available again — nothing is written anywhere else.</div>}
        <p className="small" style={{ marginBottom: 0 }}>{st.data.stats.versions} stored versions of {st.data.stats.files} files · {bytes(st.data.stats.bytes)}</p>
      </>}
    </div>
  );
}

function UpdateCheck() {
  const [state, setState] = useState<string>('');
  return (
    <>
      <button className="btn sm" onClick={async () => {
        setState('Checking…');
        try { const u = await (window as any).__pvCheckUpdate?.(); setState(u ? `Version ${u.version} is available — use the Update button at the top.` : 'You have the latest version.'); }
        catch (e: any) { setState('Could not check: ' + (e?.message || e)); }
      }}>Check for updates</button>
      <span className="small muted">{state}</span>
    </>
  );
}

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
      <Appearance />
      <EditorSaving />
      <BackupStorage />
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
          <span>Version</span><span className="row">{info.data?.version} {isTauri && <UpdateCheck />}</span>
          <span>Database schema</span><span>v{info.data?.schemaVersion} (SQLite {info.data?.sqlite})</span>
          <span>Data folder</span><span className="mono row">{info.data?.dataDir}{isTauri && info.data && <button className="btn ghost sm" onClick={() => act(() => revealPath(info.data.dataDir + '/vault.db'))}>Show</button>}</span>
          <span>Shortcuts</span><span><kbd>Ctrl K</kbd> command palette · <kbd>Ctrl Shift F</kbd> search · <kbd>Ctrl N</kbd> new project · <kbd>Ctrl B</kbd> sidebar · <kbd>Alt 1–6</kbd> project tabs · <kbd>F5</kbd> rescan · <kbd>Ctrl S</kbd> save note · <kbd>Alt ←</kbd> back</span>
        </div>
      </div>
    </div>
  );
}
