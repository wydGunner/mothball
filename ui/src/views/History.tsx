import { useMemo, useState } from 'react';
import { LuSearch, LuGitCompareArrows, LuUndo2, LuDownload, LuFileSearch, LuCamera, LuTrash2, LuEye, LuShieldCheck, LuTag } from 'react-icons/lu';
import { call, pickFolder, isTauri, revealPath } from '../ipc';
import { Modal, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { ago, bytes, cx, dateTime } from '../format';
import type { ProjectCtx } from './Project';
import { CompareDialog, RestoreDialog, REASON, isNamed, type Version } from './Versions';

interface BackupRow extends Version { fileId: string; fileName: string; currentPath: string; fileState: string; projectId: string; projectName: string }
interface Snapshot { id: string; projectId: string; name: string; description: string; at: number; fileCount: number; bytes: number; skipped: { path: string; reason: string }[] }

const DAY = 86_400_000;
const toInput = (ms?: number) => (ms ? new Date(ms - new Date(ms).getTimezoneOffset() * 60_000).toISOString().slice(0, 10) : '');
const fromInput = (s: string, end = false) => (s ? new Date(s + (end ? 'T23:59:59.999' : 'T00:00:00')).getTime() : undefined);

/** Project tab: every stored backup/version of every file, plus project-wide snapshots. */
export function HistoryTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  return (
    <div className="page">
      <p className="small muted" style={{ marginTop: 0 }}>
        <b>File backups</b> are stored copies of single files. <b>Snapshots</b> capture the whole project at a moment.
        Both live in Project Vault’s backup storage, separate from your project folder and from Git commits. The <b>Timeline</b> tab is only an activity log — it records what happened but holds no file contents.
      </p>
      <div className="histsplit">
        <BackupBrowser ctx={ctx} />
        <div>
          <Snapshots ctx={ctx} />
          <Retention projectId={pid} />
        </div>
      </div>
    </div>
  );
}

function BackupBrowser({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const [q, setQ] = useState('');
  const [namedOnly, setNamedOnly] = useState(true);
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [compare, setCompare] = useState<BackupRow | null>(null);
  const [restoreV, setRestoreV] = useState<BackupRow | null>(null);
  const toast = useToast();
  const act = useAction();
  const rows = useLoad(() => call<BackupRow[]>('versions.browse', { projectId: pid, query: q, namedOnly, from: fromInput(from), to: fromInput(to, true), limit: 500 }), [pid, q, namedOnly, from, to, ctx.scanTick]);
  const quick = (days: number) => { setFrom(toInput(Date.now() - days * DAY)); setTo(''); };
  const exportCopy = async (v: BackupRow) => {
    const dest = await pickFolder('Save a copy of this backup to…');
    if (!dest) return;
    const r: any = await act(() => call('versions.export', { id: v.id, dest }));
    if (r) toast(`Saved ${r.path}`, 'success', isTauri ? { label: 'Show', onClick: () => revealPath(r.path) } : undefined);
  };
  return (
    <section className="card pad">
      <h3 style={{ marginTop: 0 }}><LuShieldCheck /> File backups</h3>
      <div className="row" style={{ flexWrap: 'wrap', marginBottom: 8 }}>
        <div className="searchinput grow" style={{ width: 'auto', minWidth: 220 }}><LuSearch /><input className="input" placeholder="Search — file name, backup name, description, tags…" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <label className="row small" style={{ whiteSpace: 'nowrap' }}><input type="checkbox" checked={namedOnly} onChange={(e) => setNamedOnly(e.target.checked)} /> Backups only (hide automatic versions)</label>
      </div>
      <div className="row small" style={{ flexWrap: 'wrap', marginBottom: 8 }}>
        <span className="dim">From</span><input type="date" className="input" style={{ width: 150 }} value={from} onChange={(e) => setFrom(e.target.value)} aria-label="From date" />
        <span className="dim">to</span><input type="date" className="input" style={{ width: 150 }} value={to} onChange={(e) => setTo(e.target.value)} aria-label="To date" />
        <button className="chip" onClick={() => quick(1)}>24 h</button><button className="chip" onClick={() => quick(7)}>7 days</button><button className="chip" onClick={() => quick(30)}>30 days</button>
        {(from || to) && <button className="chip" onClick={() => { setFrom(''); setTo(''); }}>Any time</button>}
      </div>
      {rows.error ? <div className="alert bad">{rows.error}</div> : !rows.data ? <div className="muted small">Loading…</div> : !rows.data.length ? (
        <div className="small muted">{q || from || to ? 'No backups match.' : namedOnly ? 'No backups yet. Use “Save with backup…” in the editor (Ctrl+Shift+S) or “Back up now…” in a file’s History.' : 'No stored versions yet.'}</div>
      ) : <>
        <div className="small dim" style={{ marginBottom: 4 }}>{rows.data.length}{rows.data.length >= 500 ? '+' : ''} stored {rows.data.length === 1 ? 'copy' : 'copies'}, newest first. These are copies in backup storage, not your live files.</div>
        {rows.data.map((v) => (
          <div key={v.id} className={cx('verrow', isNamed(v) && 'named')} data-testid="backup-row">
            <div className="grow verbody">
              <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
                <span className="kindtag backup">{isNamed(v) ? 'backup' : 'auto version'}</span>
                <b className="mono">{v.name || v.fileName}</b>
                {v.name && <span className="small dim">of {v.fileName}</span>}
                {v.isCurrent && <span className="badge ok">matches current file</span>}
                {v.fileState !== 'present' && <span className="badge warn">file {v.fileState}</span>}
                {v.tags.map((t) => <span key={t} className="tag"><LuTag size={10} />{t}</span>)}
              </div>
              {v.note && <div className="small" style={{ whiteSpace: 'pre-wrap', margin: '2px 0' }}>{v.note}</div>}
              <div className="small dim">{dateTime(v.at)} ({ago(v.at)}) · {REASON[v.reason] || v.reason} · {bytes(v.size)} · {v.path}</div>
            </div>
            <div className="row veracts">
              {!v.isCurrent && v.fileState === 'present' && <button className="btn ghost sm" title="Compare this backup with the current file" onClick={() => setCompare(v)}><LuGitCompareArrows /> vs now</button>}
              {!v.isCurrent && <button className="btn ghost sm" title="Restore this backup…" onClick={() => setRestoreV(v)}><LuUndo2 /> Restore</button>}
              <button className="btn ghost sm icon" title="Save a copy as a normal file…" onClick={() => exportCopy(v)}><LuDownload /></button>
              <button className="btn ghost sm icon" title="Show this file’s full history" onClick={() => ctx.nav.go({ view: 'project', id: v.projectId, tab: 'files', focus: { type: 'file', id: v.fileId, action: 'history', versionId: v.id, nonce: Date.now() } })}><LuFileSearch /></button>
            </div>
          </div>
        ))}
      </>}
      {compare && <CompareDialog fileId={compare.fileId} title={compare.fileName} old={compare.id} new="current" onClose={() => setCompare(null)} />}
      {restoreV && <RestoreDialog v={restoreV} fileName={restoreV.fileName} absPath={undefined} present={restoreV.fileState === 'present'} onClose={() => setRestoreV(null)} onDone={() => { setRestoreV(null); rows.reload(); ctx.scan(true); }} />}
    </section>
  );
}

function Snapshots({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const list = useLoad(() => call<Snapshot[]>('snapshots.list', { projectId: pid }), [pid, ctx.scanTick]);
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState<Snapshot | null>(null);
  const [viewing, setViewing] = useState<Snapshot | null>(null);
  const confirm = useConfirm();
  const act = useAction();
  return (
    <section className="card pad" style={{ marginBottom: 16 }}>
      <div className="row"><h3 className="grow" style={{ margin: 0 }}><LuCamera /> Project snapshots</h3><button className="btn sm primary" onClick={() => setCreating(true)}><LuCamera /> Create snapshot…</button></div>
      <p className="small dim">A restore point for every file in the project. Unchanged contents are shared with file backups, so snapshots are cheap.</p>
      {list.error ? <div className="alert bad">{list.error}</div> : !list.data ? <div className="small muted">Loading…</div> : !list.data.length ? <div className="small muted">No snapshots yet.</div> :
        list.data.map((s) => (
          <div key={s.id} className="snaprow" data-testid="snapshot-row">
            <div className="grow" style={{ minWidth: 0 }}>
              <div className="row" style={{ gap: 6 }}><span className="kindtag snapshot">snapshot</span><b>{s.name}</b></div>
              {s.description && <div className="small">{s.description}</div>}
              <div className="small dim">{dateTime(s.at)} ({ago(s.at)}) · {s.fileCount} files · {bytes(s.bytes)}{s.skipped.length ? ` · ${s.skipped.length} skipped` : ''}</div>
            </div>
            <button className="btn ghost sm icon" title="Details" onClick={() => setViewing(s)}><LuEye /></button>
            <button className="btn ghost sm" onClick={() => setRestoring(s)}><LuUndo2 /> Restore…</button>
            <button className="btn ghost sm icon" title="Delete snapshot" onClick={async () => { if (await confirm({ title: 'Delete this snapshot?', danger: true, confirm: 'Delete snapshot', body: <>The restore point “<b>{s.name}</b>” is removed from backup storage. Your project files are not touched, and file backups are kept.</> })) { await act(() => call('snapshots.delete', { id: s.id }), 'Snapshot deleted'); list.reload(); } }}><LuTrash2 /></button>
          </div>
        ))}
      {creating && <CreateSnapshot projectId={pid} onClose={() => setCreating(false)} onDone={() => { setCreating(false); list.reload(); ctx.reload(); }} />}
      {restoring && <RestoreSnapshot s={restoring} root={ctx.project.rootPath} onClose={() => setRestoring(null)} onDone={() => { setRestoring(null); list.reload(); ctx.scan(true); }} />}
      {viewing && <Modal title={`Snapshot · ${viewing.name}`} onClose={() => setViewing(null)}>
        <p className="small">{dateTime(viewing.at)} · {viewing.fileCount} files · {bytes(viewing.bytes)}</p>
        {viewing.description && <p>{viewing.description}</p>}
        {viewing.skipped.length ? <><b className="small">Not included</b><ul className="small">{viewing.skipped.slice(0, 200).map((x) => <li key={x.path}><span className="mono">{x.path}</span> — {x.reason}</li>)}</ul></> : <p className="small dim">Every tracked file was included.</p>}
      </Modal>}
    </section>
  );
}

function CreateSnapshot({ projectId, onClose, onDone }: { projectId: string; onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [maxMb, setMaxMb] = useState(200);
  const [busy, setBusy] = useState(false);
  const act = useAction();
  return (
    <Modal title="Create a project snapshot" onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!name.trim() || busy} onClick={async () => { setBusy(true); const r = await act(() => call('snapshots.create', { projectId, name: name.trim(), description, maxFileMb: maxMb }), 'Snapshot created'); setBusy(false); if (r) onDone(); }}>{busy ? 'Saving…' : 'Create snapshot'}</button></>}>
      <p className="small muted" style={{ marginTop: 0 }}>The project is rescanned, then the contents of every tracked file are stored.</p>
      <div className="field"><label>Name</label><input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Working before animation rewrite" /></div>
      <div className="field"><label>Description (optional)</label><textarea className="input" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} /></div>
      <div className="field"><label>Skip files larger than (MB)</label><input className="input" type="number" min={1} style={{ width: 120 }} value={maxMb} onChange={(e) => setMaxMb(Math.max(1, Number(e.target.value) || 1))} /></div>
    </Modal>
  );
}

function RestoreSnapshot({ s, root, onClose, onDone }: { s: Snapshot; root: string; onClose: () => void; onDone: () => void }) {
  const pv = useLoad(() => call<{ changes: { path: string; action: string }[]; unchanged: number; leftAlone: string[] }>('snapshots.preview', { id: s.id }), [s.id]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<any>(null);
  const act = useAction();
  const counts = useMemo(() => {
    const c = { overwrite: 0, recreate: 0 };
    for (const x of pv.data?.changes || []) (c as any)[x.action]++;
    return c;
  }, [pv.data]);
  if (result) return (
    <Modal title="Snapshot restored" onClose={onDone} footer={<button className="btn primary" onClick={onDone}>Done</button>}>
      <p>{result.restored.length} file(s) restored and verified.</p>
      {result.errors.length > 0 && <div className="alert bad"><b>{result.errors.length} could not be restored:</b><ul className="small">{result.errors.map((e: string) => <li key={e}>{e}</li>)}</ul></div>}
      <p className="small dim">The state from just before the restore was saved as the snapshot “{result.undoSnapshot?.name}”, so you can go back.</p>
    </Modal>
  );
  return (
    <Modal wide title={`Restore snapshot “${s.name}”?`} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!pv.data || busy || !pv.data.changes.length} onClick={async () => { setBusy(true); const r = await act(() => call('snapshots.restore', { id: s.id })); setBusy(false); if (r) setResult(r); }}><LuUndo2 /> {busy ? 'Restoring…' : `Restore ${pv.data?.changes.length || 0} file(s)`}</button></>}>
      {pv.error ? <div className="alert bad">{pv.error}</div> : !pv.data ? <div className="muted">Checking the project…</div> : <>
        <p style={{ marginTop: 0 }}>From the snapshot of {dateTime(s.at)} into <span className="mono small" style={{ wordBreak: 'break-all' }}>{root}</span>:</p>
        <ul className="small">
          <li><b>{counts.overwrite}</b> file(s) will be replaced with their snapshot content</li>
          <li><b>{counts.recreate}</b> missing file(s) will be recreated</li>
          <li><b>{pv.data.unchanged}</b> already match and are left as they are</li>
          <li><b>{pv.data.leftAlone.length}</b> file(s) added since the snapshot are <b>kept</b> (never deleted)</li>
        </ul>
        {!pv.data.changes.length && <div className="alert">The project already matches this snapshot.</div>}
        {pv.data.changes.length > 0 && <details><summary className="small">Show files</summary><ul className="small mono">{pv.data.changes.slice(0, 300).map((c) => <li key={c.path}>{c.action === 'overwrite' ? 'replace' : 'recreate'} · {c.path}</li>)}</ul></details>}
        <p className="small dim">Before anything changes, every stored copy is checked, and a snapshot of the current state is taken automatically. Each restored file is verified after writing.</p>
      </>}
    </Modal>
  );
}

/** Retention: deleting old versions always previews the count and asks first. */
export function Retention({ projectId, onDone }: { projectId: string; onDone?: () => void }) {
  const stats = useLoad(() => call<any>('versions.stats', { projectId }), [projectId]);
  const [keep, setKeep] = useState(10);
  const [includeNamed, setIncludeNamed] = useState(false);
  const confirm = useConfirm();
  const act = useAction();
  const toast = useToast();
  const run = async () => {
    const dry: any = await act(() => call('versions.prune', { projectId, keep, includeNamed, dryRun: true }));
    if (!dry) return;
    if (!dry.wouldDelete) { toast('Nothing to clean up with these settings.', 'info'); return; }
    if (!(await confirm({
      title: `Delete ${dry.wouldDelete} stored version(s)?`, danger: true, confirm: `Delete ${dry.wouldDelete}`,
      body: <>For each file, the newest <b>{keep}</b> {includeNamed ? <>stored copies <b>including named backups</b></> : 'automatic versions'} are kept; older ones are deleted from backup storage. {includeNamed ? '' : 'Backups you named or saved on purpose are not touched.'} Your project files are not changed. This can’t be undone.</>,
    }))) return;
    const r: any = await act(() => call('versions.prune', { projectId, keep, includeNamed }));
    if (r) { toast(`Deleted ${r.deletedVersions} old version(s), freed ${bytes(r.freedBytes)}`, 'success'); stats.reload(); onDone?.(); }
  };
  return (
    <section className="card pad">
      <h3 style={{ marginTop: 0 }}>Storage &amp; cleanup</h3>
      {stats.data && <p className="small">Stored: <b>{stats.data.versions}</b> versions of {stats.data.files} files · <b>{bytes(stats.data.bytes)}</b> (identical contents are stored once).</p>}
      <p className="small dim">Nothing is ever deleted automatically.</p>
      <div className="row small" style={{ flexWrap: 'wrap' }}>
        Keep the newest <input className="input" type="number" min={1} style={{ width: 70, padding: '2px 6px' }} value={keep} onChange={(e) => setKeep(Math.max(1, Number(e.target.value) || 1))} /> per file
        <label className="row"><input type="checkbox" checked={includeNamed} onChange={(e) => setIncludeNamed(e.target.checked)} /> also named backups</label>
        <button className="btn sm" onClick={run}>Clean up…</button>
      </div>
    </section>
  );
}
