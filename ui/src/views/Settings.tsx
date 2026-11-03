import { useEffect, useState, type ReactElement } from 'react';
import { LuFolderOpen, LuShieldCheck, LuDatabaseBackup, LuRefreshCw, LuInfo, LuTriangleAlert, LuArchiveRestore, LuPalette, LuFileCode, LuHardDrive, LuRadar, LuShapes, LuPrinter, LuPencil, LuTrash2, LuPlus } from 'react-icons/lu';
import { SUBTYPES, saveCustomTypes, typeKeyOf, useTypes, type CustomType } from '../types';
import { PrinterSettings } from './Printers';
import { call, pickFolder, pickFiles, revealPath, isTauri } from '../ipc';
import { Modal, ProjectIcon, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { getSaveMode, setSaveMode, getLargeThreshold, setLargeThreshold, type SaveMode } from './SmartSave';
import { bytes, CATEGORY_LABELS, cx, dateTime, hiddenTypes, setHiddenTypes } from '../format';
import type { Nav } from '../main';
import { THEMES, applyTheme, getTheme, themeFromColor } from '../theme';

/** How many projects stay open as tabs (1 = one at a time, as before tabs). */
function TabSettings() {
  const [n, setN] = useState(() => { try { return Number(localStorage.getItem('pv.maxTabs')) || 10; } catch { return 10; } });
  const set = (v: number) => { setN(v); try { localStorage.setItem('pv.maxTabs', String(v)); } catch { /* ignore */ } };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Open projects</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Projects you open stay open as tabs at the top, so you can switch between them without losing your place or unsaved edits. Ctrl+Tab switches, Ctrl+W closes.</p>
      <div className="row small">Keep up to
        <select className="input" style={{ width: 'auto' }} value={n} onChange={(e) => set(Number(e.target.value))} aria-label="Open project tabs">
          <option value={1}>1 (one project at a time)</option>{[3, 5, 10].map((x) => <option key={x} value={x}>{x} projects</option>)}
        </select>
        open; the oldest one without unsaved edits closes when you open another.</div>
    </div>
  );
}

function Appearance() {
  const [cur, setCur] = useState(getTheme());
  const pick = (t: typeof cur) => { applyTheme(t); setCur(t); };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <p className="muted small" style={{ marginTop: 0 }}>The background is always black. Pick one color family for highlights: shades of one color, or neighbouring colors that go together.</p>
      <div className="row wrap" style={{ gap: 10 }}>
        {THEMES.map((t) => (
          <button key={t.id} className={'theme-card' + (cur.id === t.id ? ' on' : '')} onClick={() => pick(t)} aria-label={t.name} aria-pressed={cur.id === t.id} title={`Use the ${t.name} colors`}>
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
        <span className="small dim">Preview</span>
        <button className="btn primary sm" tabIndex={-1}>Button</button>
        <span className="pill"><span className="dot on" />Paused</span>
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
      <h2>When you press Ctrl+S</h2>
      <p className="muted small" style={{ marginTop: 0 }}>In the built-in editor. Ctrl+Shift+S always opens the save-with-backup dialog.</p>
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

/** Where managed projects ("Kept in app storage") live; moving them frees the old drive. */
function ProjectStorage() {
  const st = useLoad(() => call<any>('storage.projects'), []);
  const [pending, setPending] = useState<string | null>(null);
  const [moveExisting, setMoveExisting] = useState(true);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<any>(null);
  const act = useAction();
  const choose = async () => {
    const dir = await pickFolder('Choose where Mothball keeps project folders');
    if (!dir) return;
    st.reload(); // the dialog reads the counts live, so they update when this returns
    setPending(dir); setMoveExisting(true); setResult(null);
  };
  const apply = async (dir: string, move: boolean) => {
    setBusy(true);
    const r: any = await act(() => call('storage.setProjectsRoot', { path: dir, moveExisting: move }));
    setBusy(false);
    if (r) { setPending(null); setResult(r); st.reload(); }
  };
  const d = st.data;
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Project folders</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Where Mothball keeps the files of projects it manages for you (projects that show “Kept in app storage”). Projects linked to your own folders stay where they are.</p>
      {st.error ? <div className="alert bad">{st.error}</div> : !d ? <div className="muted small">Loading…</div> : <>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <span className="mono small grow" style={{ wordBreak: 'break-all' }}>{d.root}</span>
          {d.isDefault && <span className="badge">default</span>}
          {isTauri && <button className="btn sm icon" title="Show in Explorer" onClick={() => act(() => revealPath(d.root))}><LuFolderOpen /></button>}
          <button className="btn sm" disabled={busy} onClick={choose}>Change…</button>
          {!d.isDefault && <button className="btn sm ghost" disabled={busy} onClick={() => { st.reload(); setPending(d.defaultRoot); setMoveExisting(true); setResult(null); }}>Use default</button>}
        </div>
        <p className="small" style={{ marginBottom: 0 }}>{d.projects} project{d.projects === 1 ? '' : 's'} · {bytes(d.bytes)}{d.notHere > 0 ? ` · ${d.notHere} still in an older location (${bytes(d.bytesNotHere)})` : ''}</p>
        {d.notHere > 0 && !pending && <button className="btn sm" style={{ marginTop: 8 }} disabled={busy} onClick={() => apply(d.root, true)}>{busy ? 'Moving…' : `Move ${d.notHere === 1 ? 'it' : 'them'} here`}</button>}
      </>}
      {result && (
        <div className={cx('alert small', result.failed.length ? 'warn' : 'info')} style={{ marginTop: 10, display: 'block' }}>
          {result.moved.length > 0 ? <>Moved {result.moved.length} project{result.moved.length === 1 ? '' : 's'}{result.freed > 0 ? ` and freed ${bytes(result.freed)} in the old location` : ''}.</> : 'New projects will be created here.'}
          {result.moved.some((m: any) => m.oldFolderKept) && <div>Some old folders couldn’t be deleted (a file may be open); the projects use the new copies, so you can delete the old folders yourself.</div>}
          {result.failed.map((f: any) => <div key={f.name}><b>{f.name}</b> wasn’t moved: {f.error}</div>)}
        </div>
      )}
      {pending && d && (
        <Modal title="Move project folders?" onClose={() => !busy && setPending(null)} footer={<>
          <button className="btn" disabled={busy} onClick={() => setPending(null)}>Cancel</button>
          <button className="btn primary" disabled={busy} onClick={() => apply(pending, moveExisting)}>{busy ? 'Moving…' : moveExisting && d.projects ? 'Move projects' : 'Use this folder'}</button>
        </>}>
          <p style={{ marginTop: 0 }}>New projects will be created in <span className="mono">{pending}</span>.</p>
          {d.projects > 0 && <>
            <label className="row" style={{ alignItems: 'flex-start' }}><input type="checkbox" checked={moveExisting} onChange={(e) => setMoveExisting(e.target.checked)} style={{ marginTop: 3 }} />
              <span>Also move the {d.projects} existing project{d.projects === 1 ? '' : 's'} there ({bytes(d.bytes)})</span></label>
            {moveExisting && <p className="small muted" style={{ marginBottom: 0 }}>Each project is copied, every file is checked against the original, and only then is the old folder deleted — so the space is freed on the old drive. If anything goes wrong with a project, it stays where it is. Close programs that have project files open first.</p>}
          </>}
        </Modal>
      )}
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
    const dir = await pickFolder('Choose where Mothball keeps file backups and snapshots');
    if (!dir) return;
    if (!(await confirm({ title: 'Move backup storage?', confirm: 'Copy and switch', body: <>All stored backups and snapshots are <b>copied</b> to <span className="mono">{dir}</span> and verified. Mothball then uses the new location. The old folder is left as it is — delete it yourself once you’re happy.</> }))) return;
    setBusy(true);
    const r: any = await act(() => call('storage.set', { path: dir }));
    setBusy(false);
    if (r) { toast(`Backup storage is now ${r.root} (${r.copied} object(s) copied)`, 'success'); st.reload(); }
  };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Backup storage location</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Where file backups, automatic versions and project snapshots are stored. Each backup also gets a readable <code>.json</code> description next to it, so the folder makes sense even without Mothball.</p>
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

const BackupDest = ({ value, onChange }: { value: string; onChange: (v: string) => void }) => (
  <div className="row"><input className="input mono" value={value} onChange={(e) => onChange(e.target.value)} placeholder="E:\Backups\Mothball" /><button className="btn" onClick={async () => { const p = await pickFolder('Backup destination'); if (p) onChange(p); }}><LuFolderOpen /> Browse…</button></div>
);

function BackupCard({ nav, dataDir }: { nav: Nav; dataDir?: string }) {
  const [dest, setDest] = useState(() => localStorage.getItem('pv.backupDest') || '');
  const [scope, setScope] = useState('');
  const [files, setFiles] = useState(() => localStorage.getItem('pv.backupFiles') === '1');
  const [zip, setZip] = useState(() => localStorage.getItem('pv.backupZip') === '1');
  const [maxMb, setMaxMb] = useState(500);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<any>(null);
  const act = useAction();
  const run = async () => {
    setBusy(true);
    localStorage.setItem('pv.backupDest', dest); localStorage.setItem('pv.backupFiles', files ? '1' : '0'); localStorage.setItem('pv.backupZip', zip ? '1' : '0');
    const r = await act(() => call('backup.create', { dest, projectId: scope || null, includeFiles: files, zip, maxFileMb: maxMb }), 'Backup created and verified');
    setBusy(false);
    if (r) setResult(r);
  };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Back up now</h2>
      <p className="muted small" style={{ marginTop: 0 }}>A consistent copy of the vault database, a readable export of every project (notes as Markdown, file documentation, tasks, timeline) and a manifest with checksums. Verified right after it's written, and readable without Mothball.</p>
      <div className="field"><label>Destination folder (e.g. an external drive)</label><BackupDest value={dest} onChange={setDest} /></div>
      <div className="grid2">
        <div className="field"><label>Scope</label>
          <select className="input" title="Back up every project, or just one" value={scope} onChange={(e) => setScope(e.target.value)}><option value="">Entire vault</option>{nav.projects.map((p) => <option key={p.id} value={p.id}>Only: {p.name}</option>)}</select>
        </div>
        <div className="field"><label>Format</label>
          <select className="input" title="A folder you can browse, or one .zip file that's easy to copy around" value={zip ? 'zip' : 'folder'} onChange={(e) => setZip(e.target.value === 'zip')}><option value="folder">Folder</option><option value="zip">Single .zip file (uncompressed)</option></select>
        </div>
      </div>
      <label className="row" style={{ alignItems: 'flex-start', marginBottom: 6 }}><input type="checkbox" checked={files} onChange={(e) => setFiles(e.target.checked)} style={{ marginTop: 3 }} />
        <span><b>Include the project files themselves</b><div className="small dim">Copies every tracked file (your scripts, models, prints…) so the backup alone can bring a project back. Skip files larger than <input className="input" type="number" min={1} style={{ width: 80, padding: '1px 6px' }} value={maxMb} onChange={(e) => setMaxMb(Math.max(1, Number(e.target.value) || 1))} /> MB.</div></span></label>
      {!files && <div className="alert warn small" style={{ marginBottom: 10 }}><LuTriangleAlert /><div>Without this, project files aren't copied: linked projects are saved as references to their folders. Managed project files live in <span className="mono">{dataDir}{dataDir?.includes('\\') ? '\\' : '/'}managed</span>.</div></div>}
      <button className="btn primary" disabled={!dest || busy} onClick={run}>{busy ? 'Backing up…' : 'Create backup now'}</button>
      {result && <div className="alert info" style={{ marginTop: 12 }}><LuShieldCheck /><div className="grow">Backup written to <span className="mono" style={{ wordBreak: 'break-all' }}>{result.path}</span> — {result.verification.checkedFiles} files verified{result.projectFiles ? `, including ${result.projectFiles} project files (${bytes(result.projectBytes)})` : ''}.{result.skipped?.length > 0 && <> {result.skipped.length} project file(s) skipped: {result.skipped.slice(0, 3).map((x: any) => `${x.path || x.project} (${x.reason})`).join(', ')}{result.skipped.length > 3 ? '…' : ''}</>}</div>
        {isTauri && <button className="btn sm" onClick={() => act(() => revealPath(result.path))}>Show</button>}</div>}
    </div>
  );
}

function ScheduleCard() {
  const sched = useLoad(() => call<any>('backup.schedule'), []);
  const [s, setS] = useState<any>(null);
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  useEffect(() => { if (sched.data && !s) setS(sched.data); }, [sched.data, s]);
  if (!s) return null;
  const save = async (patch: any) => {
    const next = { ...s, ...patch };
    if (patch.keep > 0 && patch.keep !== s.keep && !(await confirm({ title: `Keep only the newest ${patch.keep} scheduled backups?`, confirm: 'Turn on cleanup', danger: true,
      body: <>After each scheduled backup, older <b>scheduled</b> backups in <span className="mono">{next.dest || 'the backup folder'}</span> beyond the newest {patch.keep} are deleted. Backups you made with “Create backup now” are never deleted.</> }))) return;
    const prev = s;
    setS(next); // optimistic, so toggles respond immediately
    const r = await act(() => call('backup.setSchedule', { schedule: next }));
    if (r) { setS(r); toast('Backup schedule saved', 'success'); } else setS(prev);
  };
  const runNow = async () => {
    const r: any = await act(() => call('backup.runDue', { force: true }));
    if (r?.result?.ok) toast(`Scheduled backup written: ${r.result.path}`, 'success'); else if (r) toast(`Scheduled backup failed: ${r.result?.error}`, 'error');
    try { setS(await call<any>('backup.schedule')); } catch { /* keep what's shown */ }
  };
  const every = s.everyDays >= 1 ? `${s.everyDays}` : 'h';
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Scheduled backups</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Runs automatically while Mothball is open (checked at start-up and every 30 minutes). If the app was closed when one was due, it runs the next time you open it.</p>
      <label className="row" style={{ marginBottom: 10 }}><input type="checkbox" checked={!!s.enabled} onChange={(e) => save({ enabled: e.target.checked })} /> <b>Back up automatically</b></label>
      <div className="field"><label>Destination folder</label><BackupDest value={s.dest} onChange={(dest) => setS({ ...s, dest })} /></div>
      <div className="grid2">
        <div className="field"><label>How often</label>
          <select className="input" title="Runs while the app is open; if it was closed when one was due, it runs on the next start" value={every} onChange={(e) => setS({ ...s, everyDays: e.target.value === 'h' ? 1 / 24 : Number(e.target.value) })}>
            <option value="h">Every hour</option><option value="1">Every day</option><option value="3">Every 3 days</option><option value="7">Every week</option><option value="30">Every month</option>
          </select></div>
        <div className="field"><label>Keep</label>
          <select className="input" title="Only backups made by the schedule are ever removed; ones you make yourself are kept" value={s.keep} onChange={(e) => setS({ ...s, keep: Number(e.target.value) })}>
            <option value={0}>Every scheduled backup (never delete)</option>{[3, 5, 10, 20, 50].map((n) => <option key={n} value={n}>Newest {n} scheduled backups</option>)}
          </select></div>
      </div>
      <label className="row small" style={{ marginBottom: 4 }}><input type="checkbox" checked={!!s.includeFiles} onChange={(e) => setS({ ...s, includeFiles: e.target.checked })} /> Include project files (up to {s.maxFileMb} MB each)</label>
      <label className="row small" style={{ marginBottom: 10 }}><input type="checkbox" checked={!!s.zip} onChange={(e) => setS({ ...s, zip: e.target.checked })} /> Save as a single .zip</label>
      <div className="row">
        <button className="btn primary" onClick={() => save({ dest: s.dest, everyDays: s.everyDays, keep: s.keep, includeFiles: s.includeFiles, zip: s.zip })}>Save schedule</button>
        <button className="btn" disabled={!s.dest} onClick={runNow} title="Make a scheduled backup right away (counts toward Keep)">Run now</button>
        <span className="small dim right">{s.lastRun ? <>Last run {dateTime(s.lastRun)} — {s.lastResult?.ok ? 'OK' : <span style={{ color: 'var(--bad)' }}>failed: {s.lastResult?.error}</span>}</> : 'Never run'}{s.enabled && s.nextRun ? ` · next ${dateTime(s.nextRun)}` : ''}</span>
      </div>
    </div>
  );
}

function RestoreCard({ nav }: { nav: Nav }) {
  const [path, setPath] = useState('');
  const [info, setInfo] = useState<any>(null);
  const [loading, setLoading] = useState(false);
  const [restoring, setRestoring] = useState<any>(null);
  const pending = useLoad(() => call<{ pending: boolean }>('backup.pendingRestore'), []);
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  const open = async (p: string) => {
    setPath(p); setInfo(null); setLoading(true);
    const r = await act(() => call('backup.inspect', { path: p }));
    setLoading(false);
    if (r) setInfo(r);
  };
  const restoreDb = async () => {
    if (!(await confirm({ title: 'Restore the vault database from this backup?', confirm: 'Restore on next start', danger: true,
      body: <>Every project, note, task, file description and setting goes back to how it was on <b>{dateTime(info.createdAt)}</b>. Changes made since then are not lost: the current database is kept in the data folder's <span className="mono">backups</span> folder as <span className="mono">before-restore-….db</span>. Your project files aren't touched. The restore happens when Mothball restarts.</> }))) return;
    const r = await act(() => call('backup.restoreDatabase', { path }));
    if (r) {
      pending.reload();
      if (isTauri) {
        if (await confirm({ title: 'Restart now?', confirm: 'Restart', body: 'The restored database is ready and will be used after a restart.' })) {
          const { relaunch } = await import('@tauri-apps/plugin-process');
          await relaunch();
        }
      } else toast('Staged. Restart Mothball to finish the restore.', 'success');
    }
  };
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2>Check or restore a backup</h2>
      {pending.data?.pending && <div className="alert warn" style={{ marginBottom: 10 }}><LuTriangleAlert /><div className="grow">A database restore is waiting and will be applied the next time Mothball starts.</div><button className="btn sm" onClick={async () => { await act(() => call('backup.cancelDatabaseRestore')); pending.reload(); }}>Cancel it</button></div>}
      <p className="muted small" style={{ marginTop: 0 }}>Choose a backup folder or .zip. It's verified (every checksum plus a database integrity check) before anything is offered.</p>
      <div className="row"><input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && path && open(path)} placeholder="E:\Backups\Mothball\Mothball-backup-…" />
        <button className="btn" title="Choose a backup folder" onClick={async () => { const p = await pickFolder('Backup folder'); if (p) open(p); }}><LuFolderOpen /> Folder…</button>
        <button className="btn" title="Choose a .zip backup" onClick={async () => { const p = await pickFiles('Backup .zip', [{ name: 'Mothball backup', extensions: ['zip'] }]); if (p[0]) open(p[0]); }}>.zip…</button>
        <button className="btn primary" title="Verify every checksum and the database before showing what can be restored" disabled={!path || loading} onClick={() => open(path)}>{loading ? 'Checking…' : 'Check'}</button></div>
      {info && <div style={{ marginTop: 12 }}>
        {info.verification.ok
          ? <div className="alert info"><LuShieldCheck /><div>Verified — {info.verification.checkedFiles} files match their checksums; database integrity check passed.</div></div>
          : <div className="alert bad"><LuTriangleAlert /><div>Problems found, so restoring is disabled:<ul>{info.verification.problems.map((p: string, i: number) => <li key={i}>{p}</li>)}</ul></div></div>}
        <div className="kv" style={{ margin: '10px 0' }}>
          <span>Made</span><span>{dateTime(info.createdAt)}{info.automatic ? ' (scheduled)' : ''} · app {info.appVersion}</span>
          <span>Contains</span><span>{info.projects.length} project(s){info.includesProjectFiles ? ' with copies of their files' : ' — metadata only, no project files'}</span>
        </div>
        <table className="list"><tbody>{info.projects.map((p: any) => (
          <tr key={p.id}>
            <td><b>{p.name}</b>{!p.existsNow && <span className="badge warn" style={{ marginLeft: 6 }}>not in the vault now</span>}<div className="small dim mono" style={{ wordBreak: 'break-all' }}>{p.restoreTo}</div></td>
            <td className="small dim" style={{ whiteSpace: 'nowrap' }}>{p.fileCopies ? `${p.fileCopies} files` : 'no file copies'}</td>
            <td style={{ textAlign: 'right' }}>{p.fileCopies > 0 && <button className="btn sm" disabled={!info.verification.ok} onClick={() => setRestoring(p)}><LuArchiveRestore /> Restore files…</button>}</td>
          </tr>))}</tbody></table>
        <div className="row" style={{ marginTop: 12 }}>
          <button className="btn" disabled={!info.verification.ok} onClick={restoreDb}><LuDatabaseBackup /> Restore the vault database…</button>
          <span className="small dim">Brings back projects, notes, tasks and file descriptions as they were.</span>
        </div>
      </div>}
      {restoring && <RestoreFilesDialog path={path} p={restoring} onClose={() => setRestoring(null)} onDone={() => { setRestoring(null); nav.refreshProjects(); }} />}
    </div>
  );
}

function RestoreFilesDialog({ path, p, onClose, onDone }: { path: string; p: any; onClose: () => void; onDone: () => void }) {
  const pv = useLoad(() => call<any>('backup.restorePreview', { path, projectId: p.id }), [path, p.id]);
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);
  const [res, setRes] = useState<any>(null);
  const act = useAction();
  const n = pv.data ? pv.data.recreate + (overwrite ? pv.data.differs : 0) : 0;
  if (res) return (
    <Modal title="Files restored" onClose={onDone} footer={<button className="btn primary" onClick={onDone}>Done</button>}>
      <p>{res.restored.length} file(s) restored into <span className="mono">{res.root}</span> and verified.{res.keptDifferent ? ` ${res.keptDifferent} file(s) that differ were left as they are.` : ''}</p>
      {res.errors.length > 0 && <div className="alert bad"><div><b>{res.errors.length} could not be restored:</b><ul className="small">{res.errors.map((e: string) => <li key={e}>{e}</li>)}</ul></div></div>}
    </Modal>
  );
  return (
    <Modal wide title={`Restore files of “${p.name}”`} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!pv.data || !n || busy} onClick={async () => { setBusy(true); const r = await act(() => call('backup.restoreFiles', { path, projectId: p.id, overwrite })); setBusy(false); if (r) setRes(r); }}>{busy ? 'Restoring…' : `Restore ${n} file(s)`}</button></>}>
      {pv.error ? <div className="alert bad">{pv.error}</div> : !pv.data ? <div className="muted">Comparing with the files on disk…</div> : <>
        <p style={{ marginTop: 0 }}>Into <span className="mono" style={{ wordBreak: 'break-all' }}>{pv.data.root}</span>{!pv.data.rootExists && ' (this folder will be created)'}:</p>
        <ul className="small">
          <li><b>{pv.data.recreate}</b> missing file(s) will be recreated</li>
          <li><b>{pv.data.differs}</b> file(s) differ from the backup — {overwrite ? <b>they'll be replaced</b> : 'left as they are'}</li>
          <li><b>{pv.data.unchanged}</b> already match</li>
          <li>Files that aren't in the backup are never touched or deleted</li>
        </ul>
        {pv.data.differs > 0 && <label className="row"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} /> Also replace the {pv.data.differs} file(s) that differ (their current content is kept in each file's history first)</label>}
        {pv.data.items.length > 0 && <details><summary className="small">Show files</summary><ul className="small mono">{pv.data.items.slice(0, 300).map((x: any) => <li key={x.path}>{x.action === 'recreate' ? 'recreate' : 'differs'} · {x.path}</li>)}</ul></details>}
      </>}
    </Modal>
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

function ProjectTypes({ nav }: { nav: Nav }) {
  const [hidden, setHidden] = useState<string[]>(hiddenTypes);
  useTypes();
  const used = new Map<string, number>();
  for (const p of nav.projects) {
    for (const k of new Set([p.category, typeKeyOf(p.category, p.module)])) used.set(k, (used.get(k) || 0) + 1);
  }
  const toggle = (k: string, on: boolean) => { const n = on ? hidden.filter((x) => x !== k) : [...hidden, k]; setHidden(n); setHiddenTypes(n); };
  const all = Object.entries(CATEGORY_LABELS);
  const shown = all.filter(([k]) => !hidden.includes(k)).length;
  const row = (k: string, label: string, icon: string, sub = false) => {
    const on = !hidden.includes(k);
    return (
      <label key={k} className={cx('typeopt', on && 'on', sub && 'sub')}>
        <input type="checkbox" checked={on} disabled={!sub && on && shown <= 1} onChange={(e) => toggle(k, e.target.checked)} />
        <ProjectIcon category={icon} size={sub ? 18 : 22} />
        <span className="grow">{label}</span>
        {used.get(k) ? <span className="small dim">{used.get(k)} project{used.get(k) === 1 ? '' : 's'}</span> : null}
      </label>
    );
  };
  return (
    <>
      <div className="card pad">
        <p className="muted small" style={{ marginTop: 0 }}>Turn off the types you don’t use so they don’t clutter the list when you add or edit a project. Projects that already use a type keep it, and it still shows for them.</p>
        <div className="typegrid">
          {all.map(([k, v]) => (
            <div key={k} className="typegroup">
              {row(k, v, k)}
              {!hidden.includes(k) && (SUBTYPES[k] || []).map((st) => row(`${k}:${st.id}`, st.label, k, true))}
            </div>
          ))}
        </div>
        <div className="row small" style={{ marginTop: 10 }}>
          <button className="btn ghost sm" onClick={() => { const n = all.map(([k]) => k).filter((k) => !used.has(k)); setHidden(n); setHiddenTypes(n); }}>Only the types I use</button>
          <button className="btn ghost sm" onClick={() => { setHidden([]); setHiddenTypes([]); }}>Show all</button>
        </div>
      </div>
      <CustomTypes nav={nav} />
    </>
  );
}

const blankType = (): CustomType => ({ id: '', name: '', base: 'general', fields: [{ key: '', label: '' }] });

/** Types the user defines: a name, a built-in type to base file handling on, and their own detail fields. */
function CustomTypes({ nav }: { nav: Nav }) {
  const types = useTypes();
  const [edit, setEdit] = useState<CustomType | null>(null);
  const act = useAction();
  const confirm = useConfirm();
  const count = (t: CustomType) => nav.projects.filter((p) => p.module?.subtype === `custom:${t.id}`).length;
  const save = async () => {
    if (!edit) return;
    const next = edit.id ? types.map((t) => (t.id === edit.id ? edit : t)) : [...types, edit];
    if (await act(() => saveCustomTypes(next), 'Type saved')) setEdit(null);
  };
  return (
    <div className="card pad" style={{ marginTop: 16 }}>
      <h2>Your own types</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Make a type for anything the built-in ones don’t fit — a Plutonium mod, a Minecraft datapack, a cosplay build — with the detail fields you want filled in. It appears in the project type list under “Your types”.</p>
      {types.map((t) => (
        <div key={t.id} className="printer-set-row">
          <ProjectIcon category={t.base} size={20} />
          <div className="grow" style={{ minWidth: 0 }}>
            <b>{t.name}</b> <span className="small dim">based on {CATEGORY_LABELS[t.base]} · {t.fields.length} field{t.fields.length === 1 ? '' : 's'}{count(t) ? ` · ${count(t)} project${count(t) === 1 ? '' : 's'}` : ''}</span>
          </div>
          <button className="btn ghost sm icon" title="Edit" aria-label={`Edit ${t.name}`} onClick={() => setEdit(JSON.parse(JSON.stringify(t)))}><LuPencil /></button>
          <button className="btn ghost sm icon" title="Delete" aria-label={`Delete ${t.name}`} onClick={async () => {
            if (await confirm({ title: `Delete “${t.name}”?`, confirm: 'Delete type', danger: true, body: <>{count(t) ? `${count(t)} project${count(t) === 1 ? '' : 's'} use it; they become ${CATEGORY_LABELS[t.base]} and keep what was filled in, but the fields won't show. ` : ''}This can’t be undone.</> })) {
              await act(() => saveCustomTypes(types.filter((x) => x.id !== t.id)));
            }
          }}><LuTrash2 /></button>
        </div>
      ))}
      {edit ? (
        <div className="printer-form" style={{ gridTemplateColumns: '1fr 1fr' }}>
          <div className="field"><label>Name</label><input className="input" value={edit.name} placeholder="Plutonium T6 mod" onChange={(e) => setEdit({ ...edit, name: e.target.value })} /></div>
          <div className="field"><label>Based on</label>
            <select className="input" value={edit.base} onChange={(e) => setEdit({ ...edit, base: e.target.value })}>{Object.entries(CATEGORY_LABELS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
          </div>
          <div className="small dim" style={{ gridColumn: '1 / -1', marginTop: -4 }}>“Based on” decides how files are grouped and which files get earlier versions kept.</div>
          <div style={{ gridColumn: '1 / -1' }}>
            <label className="small" style={{ fontWeight: 600 }}>Detail fields</label>
            {edit.fields.map((f, i) => (
              <div key={i} className="row" style={{ marginTop: 6 }}>
                <input className="input grow" value={f.label} placeholder={i === 0 ? 'e.g. Server name' : 'Field name'} aria-label={`Field ${i + 1}`}
                  onChange={(e) => setEdit({ ...edit, fields: edit.fields.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)) })} />
                <label className="row small" style={{ gap: 4, whiteSpace: 'nowrap' }}><input type="checkbox" checked={!!f.multiline} onChange={(e) => setEdit({ ...edit, fields: edit.fields.map((x, j) => (j === i ? { ...x, multiline: e.target.checked } : x)) })} /> Long text</label>
                <button className="btn ghost sm icon" title="Remove field" aria-label={`Remove field ${i + 1}`} onClick={() => setEdit({ ...edit, fields: edit.fields.filter((_, j) => j !== i) })}><LuTrash2 /></button>
              </div>
            ))}
            <button className="btn ghost sm" style={{ marginTop: 6 }} onClick={() => setEdit({ ...edit, fields: [...edit.fields, { key: '', label: '' }] })}><LuPlus /> Add field</button>
          </div>
          <div className="row" style={{ gridColumn: '1 / -1', justifyContent: 'flex-end' }}>
            <button className="btn" onClick={() => setEdit(null)}>Cancel</button>
            <button className="btn primary" disabled={!edit.name.trim()} onClick={save}>{edit.id ? 'Save type' : 'Create type'}</button>
          </div>
        </div>
      ) : <button className="btn" style={{ marginTop: 10 }} onClick={() => setEdit(blankType())}><LuPlus /> New type</button>}
    </div>
  );
}

function TimeSettings() {
  const sum = useLoad(() => call<{ estimate: boolean }>('time.summary', { since: Date.now() }), []);
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => { if (sum.data) setOn(sum.data.estimate); }, [sum.data]);
  const act = useAction();
  if (on === null) return null;
  return (
    <div className="card pad">
      <h2>Time tracking</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Each project’s Overview shows time spent on it: time you log or run the timer for, plus — if this is on — work sessions estimated from when the project’s files were saved, so time in Blender, Vegas, your slicer or an IDE counts too. Saves within 30 minutes of each other count as one session (plus 10 minutes before the first save). Overlapping time is only counted once.</p>
      <label className="row"><input type="checkbox" checked={on} onChange={async (e) => { const v = e.target.checked; setOn(v); await act(() => call('time.setEstimate', { on: v })); }} /> Estimate time from file activity</label>
    </div>
  );
}

function WatchCard() {
  const w = useLoad(() => call<{ enabled: boolean; seconds: number }>('watch.settings'), []);
  const [cur, setCur] = useState<{ enabled: boolean; seconds: number } | null>(null);
  useEffect(() => { if (w.data) setCur(w.data); }, [w.data]);
  const act = useAction();
  const save = async (patch: Partial<{ enabled: boolean; seconds: number }>) => {
    if (!cur) return;
    setCur({ ...cur, ...patch });
    const r = await act(() => call<any>('watch.setSettings', { patch }));
    if (r) setCur(r);
  };
  if (!cur) return null;
  return (
    <div className="card pad">
      <h2>Watch project folders</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Checks every project’s folder in the background, so edits you make in other programs (Blender, Vegas, your slicer, an IDE) land on the timeline within seconds, and the dashboard stays current. Only file sizes and dates are compared; contents are read only for files that changed. Projects whose folders are large are checked less often.</p>
      <label className="row"><input type="checkbox" checked={cur.enabled} onChange={(e) => save({ enabled: e.target.checked })} /> Watch all projects for changes</label>
      <div className="row small" style={{ marginTop: 10, opacity: cur.enabled ? 1 : 0.5 }}>
        Check every
        <select className="input" style={{ width: 'auto' }} disabled={!cur.enabled} value={cur.seconds} onChange={(e) => save({ seconds: Number(e.target.value) })}>
          {[5, 10, 15, 30, 60, 120, 300].map((n) => <option key={n} value={n}>{n < 60 ? `${n} seconds` : `${n / 60} minute${n > 60 ? 's' : ''}`}</option>)}
        </select>
      </div>
      {!cur.enabled && <p className="small dim" style={{ marginBottom: 0 }}>When off, only the open project is rescanned (every minute and when you switch back to the app). Other projects catch up when you open them.</p>}
    </div>
  );
}

type Section = 'appearance' | 'types' | 'editor' | 'tracking' | 'printers' | 'backups' | 'restore' | 'storage' | 'about';
const SECTIONS: { id: Section; label: string; icon: ReactElement; hint: string }[] = [
  { id: 'appearance', label: 'Appearance', icon: <LuPalette />, hint: 'Accent colors' },
  { id: 'types', label: 'Project types', icon: <LuShapes />, hint: 'Which project types you can pick' },
  { id: 'editor', label: 'Editor & saving', icon: <LuFileCode />, hint: 'What Ctrl+S does' },
  { id: 'tracking', label: 'Change tracking', icon: <LuRadar />, hint: 'Notice changes to project folders as they happen' },
  { id: 'printers', label: 'Printers', icon: <LuPrinter />, hint: 'Watch a Bambu Lab printer and log prints to projects' },
  { id: 'backups', label: 'Backups', icon: <LuDatabaseBackup />, hint: 'Back up now, or on a schedule' },
  { id: 'restore', label: 'Restore', icon: <LuArchiveRestore />, hint: 'Check a backup and bring things back' },
  { id: 'storage', label: 'Storage', icon: <LuHardDrive />, hint: 'Where project folders, file backups and snapshots live' },
  { id: 'about', label: 'About & updates', icon: <LuInfo />, hint: 'Version, data folder, shortcuts' },
];

export function SettingsView({ nav }: { nav: Nav }) {
  const info = useLoad(() => call<any>('app.info'), []);
  const act = useAction();
  const [sec, setSecRaw] = useState<Section>(() => (localStorage.getItem('pv.settingsSection') as Section) || 'appearance');
  const setSec = (x: Section) => { setSecRaw(x); try { localStorage.setItem('pv.settingsSection', x); } catch { /* ignore */ } };
  const cur = SECTIONS.find((x) => x.id === sec) || SECTIONS[0];
  return (
    <div className="page settings">
      <nav className="set-nav" aria-label="Settings sections">
        <h1>Settings</h1>
        {SECTIONS.map((x) => (
          <button key={x.id} className={cx('set-link', sec === x.id && 'on')} aria-current={sec === x.id ? 'page' : undefined} title={x.hint} onClick={() => setSec(x.id)}>{x.icon}<span>{x.label}</span></button>
        ))}
      </nav>
      <div className="set-body" key={sec}>
        <h2 className="set-title">{cur.label}</h2>
        {sec === 'appearance' && <><Appearance /><TabSettings /></>}
        {sec === 'types' && <ProjectTypes nav={nav} />}
        {sec === 'editor' && <EditorSaving />}
        {sec === 'tracking' && <><WatchCard /><TimeSettings /></>}
        {sec === 'printers' && <PrinterSettings />}
        {sec === 'backups' && <><BackupCard nav={nav} dataDir={info.data?.dataDir} /><ScheduleCard /></>}
        {sec === 'restore' && <RestoreCard nav={nav} />}
        {sec === 'storage' && <><ProjectStorage /><BackupStorage /></>}
        {sec === 'about' && <>
          <div className="card pad">
            <h2>Version</h2>
            <div className="kv">
              <span>App</span><span className="row">{info.data?.version} {isTauri && <UpdateCheck />}</span>
              <span>Database</span><span>schema v{info.data?.schemaVersion}, SQLite {info.data?.sqlite}</span>
              <span>Data folder</span><span className="mono row" style={{ wordBreak: 'break-all' }}>{info.data?.dataDir}{isTauri && info.data && <button className="btn ghost sm" title="Show the data folder in Explorer" onClick={() => act(() => revealPath(info.data.dataDir + '/vault.db'))}>Show</button>}</span>
            </div>
          </div>
          <div className="card pad">
            <h2>Keyboard shortcuts</h2>
            <table className="shortcuts"><tbody>
              {([['Ctrl K', 'Search everything, run a command'], ['Ctrl Shift F', 'Full search results'], ['Ctrl N', 'New project'], ['Ctrl B', 'Show or hide the sidebar'], ['Alt 1–7', 'Switch project tabs'], ['F5', 'Rescan the project'], ['Ctrl S', 'Save (see Editor & saving)'], ['Ctrl Shift S', 'Save with a backup'], ['F12', 'Go to a script function’s definition'], ['Shift F10', 'Open the context menu'], ['Alt ←', 'Back']] as const).map(([k, d]) => <tr key={k}><td><kbd>{k}</kbd></td><td>{d}</td></tr>)}
            </tbody></table>
          </div>
          <div className="card pad">
            <h2>Maintenance</h2>
            <p className="muted small" style={{ marginTop: 0 }}>Rebuild the search index if search results look out of date.</p>
            <button className="btn" onClick={() => act(() => call('search.rebuild'), 'Search index rebuilt')}><LuRefreshCw /> Rebuild search index</button>
          </div>
        </>}
      </div>
    </div>
  );
}
