import { useEffect, useMemo, useState } from 'react';
import { LuSave, LuChevronDown, LuGitCompareArrows, LuUndo2, LuTrash2, LuColumns2, LuRows3, LuSearch, LuDownload, LuPencilLine, LuShieldCheck, LuTag } from 'react-icons/lu';
import { call, pickFolder, isTauri, revealPath } from '../ipc';
import { Modal, TagInput, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { MonacoDiff } from '../components/CodeEditor';
import { ago, bytes, cx, dateTime } from '../format';
import { ChangeDiff, type CompareReq } from './Timeline';

export interface Version { id: string; sha256: string; size: number; path: string; reason: string; note: string; name: string; tags: string[]; at: number; isCurrent: boolean }

export const REASON: Record<string, string> = {
  baseline: 'first seen', auto: 'auto · changed on disk', manual: 'backup', 'before-restore': 'kept before a restore', 'before-edit': 'before an edit in Mothball',
  'before-delete': 'kept before delete', 'pre-save': 'backup · before saving', 'post-save': 'backup · saved version',
};
export const isNamed = (v: { name: string; note: string; reason: string }) => !!v.name || !!v.note || ['manual', 'pre-save', 'post-save'].includes(v.reason);

/** Version history of one file: named backups and automatic versions. */
export function VersionsPanel({ fileId, fileName, absPath, present, refreshKey, highlight, onRestored }: { fileId: string; fileName: string; absPath?: string; present: boolean; refreshKey?: string; highlight?: string; onRestored: () => void }) {
  const versions = useLoad(() => call<Version[]>('versions.list', { fileId }), [fileId, refreshKey]);
  const [q, setQ] = useState('');
  const [namedOnly, setNamedOnly] = useState(false);
  const [pick, setPick] = useState<string[]>([]);
  const [compare, setCompare] = useState<{ old: string; new: string } | null>(null);
  const [restoreV, setRestoreV] = useState<Version | null>(null);
  const [editV, setEditV] = useState<Version | null>(null);
  const [backupNow, setBackupNow] = useState(false);
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  const all = versions.data || [];
  const list = useMemo(() => all.filter((v) => (!namedOnly || isNamed(v)) && (!q || `${v.name} ${v.note} ${v.tags.join(' ')} ${v.path} ${REASON[v.reason] || v.reason}`.toLowerCase().includes(q.toLowerCase()))), [all, q, namedOnly]);
  useEffect(() => { if (highlight) document.querySelector(`[data-vid="${highlight}"]`)?.scrollIntoView({ block: 'center' }); }, [highlight, versions.data]);

  const toggle = (id: string) => setPick((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p.slice(-1), id]));
  const ordered = (a: string, b: string) => {
    const ia = all.findIndex((x) => x.id === a), ib = all.findIndex((x) => x.id === b);
    return ia > ib ? { old: a, new: b } : { old: b, new: a }; // newest first
  };
  const exportCopy = async (v: Version) => {
    const dest = await pickFolder('Save a copy of this version to…');
    if (!dest) return;
    const r: any = await act(() => call('versions.export', { id: v.id, dest }));
    if (r) toast(`Saved ${r.path}`, 'success', isTauri ? { label: 'Show', onClick: () => revealPath(r.path) } : undefined);
  };

  return (
    <div>
      <div className="row" style={{ marginBottom: 8 }}>
        <div className="searchinput grow" style={{ width: 'auto' }}><LuSearch /><input className="input" placeholder="Search backups — name, description, tags…" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <label className="row small" style={{ whiteSpace: 'nowrap' }}><input type="checkbox" checked={namedOnly} onChange={(e) => setNamedOnly(e.target.checked)} /> Backups only</label>
        {present && <button className="btn sm" onClick={() => setBackupNow(true)}><LuShieldCheck /> Back up now…</button>}
      </div>
      {all.length === 0 ? <div className="small muted">No stored versions yet. Use “Back up now…”, or “Save with backup…” in the editor. Automatic versions are kept for code/text and this project type's key files (Edit → Files &amp; versions).</div> : <>
        <div className="small dim" style={{ marginBottom: 4 }}>Newest first. Tick two to compare them, or compare one with the current file.</div>
        {list.map((v) => (
          <div key={v.id} data-vid={v.id} className={cx('verrow', pick.includes(v.id) && 'sel', highlight === v.id && 'flashv', isNamed(v) && 'named')}>
            <input type="checkbox" checked={pick.includes(v.id)} onChange={() => toggle(v.id)} aria-label="Select for comparison" />
            <div className="grow verbody">
              <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
                {v.name ? <b className="mono">{v.name}</b> : <b>{REASON[v.reason] || v.reason}</b>}
                {v.isCurrent && <span className="badge ok">matches current file</span>}
                {v.tags.map((t) => <span key={t} className="tag"><LuTag size={10} />{t}</span>)}
              </div>
              {v.note && <div className="small" style={{ whiteSpace: 'pre-wrap', margin: '2px 0' }}>{v.note}</div>}
              <div className="small dim"><span title={dateTime(v.at)}>{dateTime(v.at)} ({ago(v.at)})</span> · {v.name ? (REASON[v.reason] || v.reason) + ' · ' : ''}{bytes(v.size)}{v.path ? ` · ${v.path}` : ''}</div>
            </div>
            <div className="row veracts">
              {!v.isCurrent && present && <button className="btn ghost sm" title="Compare with the current file" onClick={() => setCompare({ old: v.id, new: 'current' })}><LuGitCompareArrows /> vs now</button>}
              {!v.isCurrent && <button className="btn ghost sm" title="Restore this version…" onClick={() => setRestoreV(v)}><LuUndo2 /> Restore</button>}
              <button className="btn ghost sm icon" title="Save a copy as a normal file…" onClick={() => exportCopy(v)}><LuDownload /></button>
              <button className="btn ghost sm icon" title="Edit name, description and tags" onClick={() => setEditV(v)}><LuPencilLine /></button>
              <button className="btn ghost sm icon" title="Delete this stored version" onClick={async () => { if (await confirm({ title: 'Delete this stored version?', danger: true, confirm: 'Delete version', body: <>Only Mothball's stored copy{v.name ? <> “<b>{v.name}</b>”</> : ''} is deleted. The file on disk is not touched.</> })) { await act(() => call('versions.delete', { id: v.id })); versions.reload(); } }}><LuTrash2 /></button>
            </div>
          </div>
        ))}
        {!list.length && <div className="small muted">No versions match.</div>}
        {pick.length === 2 && <button className="btn primary sm" style={{ marginTop: 8 }} onClick={() => setCompare(ordered(pick[0], pick[1]))}><LuGitCompareArrows /> Compare selected</button>}
      </>}
      {compare && <CompareDialog fileId={fileId} title={fileName} old={compare.old} new={compare.new} onClose={() => setCompare(null)} />}
      {restoreV && <RestoreDialog v={restoreV} fileName={fileName} absPath={absPath} present={present} onClose={() => setRestoreV(null)} onDone={() => { setRestoreV(null); versions.reload(); onRestored(); }} />}
      {editV && <EditBackupDialog v={editV} onClose={() => setEditV(null)} onDone={() => { setEditV(null); versions.reload(); }} />}
      {backupNow && <BackupNowDialog fileId={fileId} fileName={fileName} onClose={() => setBackupNow(false)} onDone={() => { setBackupNow(false); versions.reload(); }} />}
    </div>
  );
}

function BackupNowDialog({ fileId, fileName, onClose, onDone }: { fileId: string; fileName: string; onClose: () => void; onDone: () => void }) {
  const [description, setDescription] = useState('');
  const [name, setName] = useState('');
  const [edited, setEdited] = useState(false);
  const [tags, setTags] = useState<string[]>([]);
  const act = useAction();
  useEffect(() => {
    if (edited) return;
    const t = setTimeout(() => call<any>('versions.suggestName', { fileId, description }).then((r) => setName(r.name)).catch(() => {}), 250);
    return () => clearTimeout(t);
  }, [description, edited, fileId]);
  return (
    <Modal title={`Back up ${fileName}`} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" onClick={async () => { if (await act(() => call('versions.backupNow', { fileId, name, description, tags: tags.join(', ') }), 'Backup saved')) onDone(); }}><LuShieldCheck /> Save backup</button></>}>
      <p className="small muted" style={{ marginTop: 0 }}>Saves the file exactly as it is on disk right now.</p>
      <div className="field"><label>Why are you keeping this version?</label><textarea className="input" rows={3} autoFocus value={description} onChange={(e) => setDescription(e.target.value)} placeholder="e.g. Last known working version before server testing" /></div>
      <div className="field"><label>Backup name</label><input className="input mono" value={name} onChange={(e) => { setName(e.target.value); setEdited(true); }} /></div>
      <div className="field"><label>Tags (optional)</label><TagInput value={tags} onChange={setTags} /></div>
    </Modal>
  );
}

function EditBackupDialog({ v, onClose, onDone }: { v: Version; onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState(v.name);
  const [note, setNote] = useState(v.note);
  const [tags, setTags] = useState<string[]>(v.tags);
  const act = useAction();
  return (
    <Modal title="Backup details" onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" onClick={async () => { if (await act(() => call('versions.update', { id: v.id, name, description: note, tags: tags.join(', ') }))) onDone(); }}>Save</button></>}>
      <div className="field"><label>Name</label><input className="input mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. weapon_tuner_server_test.gsc" /></div>
      <div className="field"><label>Description</label><textarea className="input" rows={4} value={note} onChange={(e) => setNote(e.target.value)} /></div>
      <div className="field"><label>Tags</label><TagInput value={tags} onChange={setTags} /></div>
      <p className="small dim">Stored {dateTime(v.at)} · {bytes(v.size)} · SHA-256 {v.sha256.slice(0, 12)}…</p>
    </Modal>
  );
}

export function RestoreDialog({ v, fileName, absPath, present, onClose, onDone }: { v: Version; fileName: string; absPath?: string; present: boolean; onClose: () => void; onDone: () => void }) {
  const [keep, setKeep] = useState(true);
  const [busy, setBusy] = useState(false);
  const act = useAction();
  return (
    <Modal title="Restore this version?" onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={busy} onClick={async () => { setBusy(true); const r: any = await act(() => call('versions.restoreOpts', { id: v.id, keepCurrent: keep })); setBusy(false); if (r) onDone(); }}><LuUndo2 /> {busy ? 'Restoring…' : 'Restore'}</button></>}>
      <div className="ss-file">
        <div><span className="dim small">From</span><span><b className="mono">{v.name || REASON[v.reason] || v.reason}</b><div className="small dim">{dateTime(v.at)} · {bytes(v.size)}</div>{v.note && <div className="small">{v.note}</div>}</span></div>
        <div><span className="dim small">To</span><span className="mono small" style={{ wordBreak: 'break-all' }}>{absPath || v.path}</span></div>
      </div>
      <p>{present ? <>The current <b>{fileName}</b> on disk will be <b>replaced</b> with this version.</> : <>This recreates <b>{fileName}</b> at that location.</>} The written file is checked against the stored copy afterwards, and the project is rescanned.</p>
      {present && <label className="row"><input type="checkbox" checked={keep} onChange={(e) => setKeep(e.target.checked)} /> Keep a backup of the current file first (recommended)</label>}
      <p className="small dim">Close the file in other programs before restoring.</p>
    </Modal>
  );
}

/** Compare two sides of a file. Without explicit sides: previous version vs current. */
export function CompareDialog({ fileId, title, old, new: nw, onClose }: { fileId: string; title: string; old?: string; new?: string; onClose: () => void; onRestored?: () => void }) {
  const [sides, setSides] = useState<{ old: string; new: string } | null>(old && nw ? { old, new: nw } : null);
  const [err, setErr] = useState('');
  useEffect(() => {
    if (sides) return;
    call<Version[]>('versions.list', { fileId }).then((vs) => {
      const prev = vs.find((v) => !v.isCurrent);
      if (prev) setSides({ old: prev.id, new: 'current' }); else setErr('There is no earlier version of this file stored yet.');
    }, (e) => setErr(e.message));
  }, [fileId, sides]);
  const res = useLoad(() => (sides ? call<any>('versions.compare', { fileId, old: sides.old, new: sides.new }) : Promise.resolve(null)), [fileId, sides?.old, sides?.new]);
  const [mode, setMode] = useState<'unified' | 'split'>(() => (localStorage.getItem('pv.diffmode') as any) || 'split');
  const [noMonaco, setNoMonaco] = useState(false);
  const setM = (m: 'unified' | 'split') => { setMode(m); localStorage.setItem('pv.diffmode', m); };
  const r = res.data;
  return (
    <Modal wide title={<>Changes in <span className="mono">{title}</span></>} onClose={onClose}>
      {err ? <div className="muted">{err}</div> : res.error ? <div className="alert bad">{res.error}</div> : !r ? <div className="muted">Comparing…</div> : <>
        <div className="row small" style={{ marginBottom: 10, flexWrap: 'wrap' }}>
          <span className="pill del" title={r.old.sha256}>{r.old.label}</span>→<span className="pill add" title={r.new.sha256}>{r.new.label}</span>
          {r.kind === 'text' && !r.diff.identical && !r.diff.tooLarge && <span className="grow"><span style={{ color: 'var(--accent)' }}>+{r.diff.added}</span> <span style={{ color: 'var(--accent3)' }}>−{r.diff.removed}</span> lines</span>}
          {r.kind === 'text' && <div className="seg right"><button className={cx(mode === 'split' && 'on')} onClick={() => setM('split')}><LuColumns2 /> Side by side</button><button className={cx(mode === 'unified' && 'on')} onClick={() => setM('unified')}><LuRows3 /> Unified</button></div>}
        </div>
        {r.kind === 'text'
          ? (r.diff.identical ? <div className="muted">The two versions are identical.</div>
            : !noMonaco ? <MonacoDiff original={r.oldText} modified={r.newText} ext={r.ext || ''} sideBySide={mode === 'split'} onUnavailable={() => setNoMonaco(true)} />
            : r.diff.tooLarge ? <div className="muted">The files differ in too many places to show line by line.</div>
            : <StructuredDiff hunks={r.diff.hunks} mode={mode} />)
          : <BinaryCompare r={r} />}
      </>}
    </Modal>
  );
}

function BinaryCompare({ r }: { r: any }) {
  const dims = (d: any) => (d ? `${d[0]} × ${d[1]} × ${d[2]} mm` : '—');
  return (
    <div>
      <p className="muted small" style={{ marginTop: 0 }}>{r.identical ? 'The two versions have identical content.' : 'This is a binary file, so it can’t be compared line by line. Here is what differs:'}</p>
      <table className="list"><thead><tr><th /><th>{r.old.label}</th><th>{r.new.label}</th></tr></thead>
        <tbody>
          <tr><td className="muted">Size</td><td>{bytes(r.old.size)}</td><td>{bytes(r.new.size)} {r.new.size !== r.old.size && <span className="dim">({r.new.size > r.old.size ? '+' : '−'}{bytes(Math.abs(r.new.size - r.old.size))})</span>}</td></tr>
          {(r.oldDims || r.newDims) && <tr><td className="muted">Model bounds</td><td>{dims(r.oldDims)}</td><td>{dims(r.newDims)}</td></tr>}
          <tr><td className="muted">SHA-256</td><td className="mono small">{r.old.sha256.slice(0, 16)}…</td><td className="mono small">{r.new.sha256.slice(0, 16)}…</td></tr>
        </tbody>
      </table>
      <p className="small dim">Use “Restore” on an older version to get it back, or export both copies and open them in Blender/your slicer to compare visually.</p>
    </div>
  );
}

interface DLine { t: ' ' | '+' | '-'; text: string; o?: number; n?: number }

export function StructuredDiff({ hunks, mode }: { hunks: { lines: DLine[] }[]; mode: 'unified' | 'split' }) {
  const rows = useMemo(() => {
    if (mode === 'unified') return null;
    return hunks.map((h) => {
      const out: { l?: DLine; r?: DLine }[] = [];
      let dels: DLine[] = [], adds: DLine[] = [];
      const flush = () => { const n = Math.max(dels.length, adds.length); for (let i = 0; i < n; i++) out.push({ l: dels[i], r: adds[i] }); dels = []; adds = []; };
      for (const l of h.lines) {
        if (l.t === '-') dels.push(l);
        else if (l.t === '+') adds.push(l);
        else { flush(); out.push({ l, r: l }); }
      }
      flush();
      return out;
    });
  }, [hunks, mode]);
  return (
    <div className="sdiff">
      {hunks.map((h, hi) => (
        <div key={hi} className="hunk">
          {hi > 0 && <div className="hunksep">⋯</div>}
          {mode === 'unified'
            ? h.lines.map((l, i) => (
              <div key={i} className={cx('dl', l.t === '+' && 'add', l.t === '-' && 'del')}>
                <span className="ln">{l.o ?? ''}</span><span className="ln">{l.n ?? ''}</span><span className="mk">{l.t}</span><span className="tx">{l.text || ' '}</span>
              </div>))
            : rows![hi].map((r, i) => (
              <div key={i} className="dl sbs">
                <span className={cx('half', r.l?.t === '-' && 'del', !r.l && 'blank')}><span className="ln">{r.l?.o ?? ''}</span><span className="tx">{r.l ? r.l.text || ' ' : ''}</span></span>
                <span className={cx('half', r.r?.t === '+' && 'add', !r.r && 'blank')}><span className="ln">{r.r?.n ?? ''}</span><span className="tx">{r.r ? r.r.text || ' ' : ''}</span></span>
              </div>))}
        </div>
      ))}
    </div>
  );
}

interface HistEntry { source: string; kind: string; title: string; description?: string; versionId?: string; at: number; sha256?: string; restorable: boolean; commit?: string; from?: string; to?: string }

export const SOURCE_LABEL: Record<string, string> = {
  backup: 'Backup', 'auto-version': 'Auto version', filesystem: 'Detected on disk', 'project-vault': 'Mothball action', git: 'Git commit', notes: 'Notes',
};
const EVENT_LABEL: Record<string, string> = {
  added: 'Added', modified: 'Changed on disk', moved: 'Moved / renamed', missing: 'Went missing', restored: 'Reappeared', renamed: 'Renamed in Mothball',
  deleted: 'Deleted (Recycle Bin)', 'restored-version': 'Restored from a stored version', imported: 'Imported',
};

/** Per-file timeline. Every entry is labelled with where it came from; reasons are never inferred. */
export function FileTimeline({ fileId, fileName, absPath, path, projectId, present, refreshKey, onRestored, openFile }: { fileId: string; fileName: string; absPath?: string; path: string; projectId: string; present: boolean; refreshKey?: string; onRestored: () => void; openFile?: (id: string) => void }) {
  const hist = useLoad(() => call<HistEntry[]>('versions.history', { fileId }), [fileId, refreshKey]);
  const vers = useLoad(() => call<Version[]>('versions.list', { fileId }), [fileId, refreshKey]);
  const git = useLoad(() => call<any[]>('git.log', { projectId, path, limit: 50 }).catch(() => []), [projectId, path, refreshKey]);
  const [compare, setCompare] = useState<string | null>(null);
  const [restoreV, setRestoreV] = useState<Version | null>(null);
  const [src, setSrc] = useState('');
  const [openDiff, setOpenDiff] = useState('');
  const [cmpReq, setCmpReq] = useState<CompareReq | null>(null);
  const entries = useMemo(() => {
    const out: HistEntry[] = [...(hist.data || [])];
    for (const c of git.data || []) out.push({ source: 'git', kind: 'commit', title: c.subject, description: `${c.short} · ${c.author}`, at: c.time, restorable: false, commit: c.hash });
    return out.sort((a, b) => b.at - a.at);
  }, [hist.data, git.data]);
  if (hist.error) return <div className="alert bad">{hist.error}</div>;
  if (!hist.data) return <div className="small muted">Loading…</div>;
  const shown = entries.filter((e) => !src || e.source === src);
  const sources = [...new Set(entries.map((e) => e.source))];
  return (
    <div>
      {sources.length > 1 && <div className="row small" style={{ marginBottom: 6, flexWrap: 'wrap', gap: 4 }}>
        <button className={cx('chip', !src && 'on')} onClick={() => setSrc('')}>All</button>
        {sources.map((s) => <button key={s} className={cx('chip', src === s && 'on')} onClick={() => setSrc(s)}>{SOURCE_LABEL[s] || s}</button>)}
      </div>}
      {!shown.length && <div className="small muted">Nothing recorded for this file yet.</div>}
      <div className="tline">
        {shown.map((e, i) => {
          const v = e.versionId ? vers.data?.find((x) => x.id === e.versionId) : undefined;
          const title = e.source === 'filesystem' || e.source === 'project-vault' ? (EVENT_LABEL[e.kind] || e.kind) : e.source === 'auto-version' || (e.source === 'backup' && e.title === e.kind) ? (REASON[e.kind] || e.kind) : e.title;
          const diffKey = e.from && e.to ? `${e.at}:${e.to}` : '';
          const diffOpen = !!diffKey && openDiff === diffKey; // collapsed until asked
          return (
            <div key={i}>
            <div className={cx('tl-row', `src-${e.source}`)}>
              <span className={cx('srcbadge', `src-${e.source}`)}>{SOURCE_LABEL[e.source] || e.source}</span>
              <div className="grow" style={{ minWidth: 0 }}>
                <div><b className={cx(e.source === 'backup' && 'mono')}>{title}</b>{(e.source === 'filesystem' || e.source === 'project-vault') && e.title && <span className="small dim"> — {e.title}</span>}</div>
                {e.description && <div className="small" style={{ whiteSpace: 'pre-wrap' }}>{e.description}</div>}
                <div className="small dim" title={dateTime(e.at)}>{dateTime(e.at)} · {ago(e.at)}{e.restorable ? ' · contents stored' : ''}</div>
              </div>
              {e.restorable && v && <div className="row" style={{ gap: 2 }}>
                {!v.isCurrent && present && <button className="btn sm" onClick={() => setCompare(v.id)}><LuGitCompareArrows /> vs now</button>}
                {!v.isCurrent && <button className="btn sm" onClick={() => setRestoreV(v)}><LuUndo2 /> Restore</button>}
                {v.isCurrent && <span className="badge ok">matches current file</span>}
              </div>}
              {diffKey && <RevealButton open={diffOpen} onClick={() => setOpenDiff(diffOpen ? '' : diffKey)} />}
            </div>
            {diffOpen && <ChangeDiff fileId={fileId} from={e.from!} to={e.to!} title={fileName} openFile={openFile || (() => {})} onCompare={setCmpReq} />}
            </div>
          );
        })}
      </div>
      <p className="small dim">Activity entries record that something happened; they don’t hold file contents and can’t be restored. Backups and auto versions can.</p>
      {compare && <CompareDialog fileId={fileId} title={fileName} old={compare} new="current" onClose={() => setCompare(null)} />}
      {cmpReq && <CompareDialog fileId={cmpReq.fileId} title={cmpReq.title} old={cmpReq.old} new={cmpReq.new} onClose={() => setCmpReq(null)} />}
      {restoreV && <RestoreDialog v={restoreV} fileName={fileName} absPath={absPath} present={present} onClose={() => setRestoreV(null)} onDone={() => { setRestoreV(null); hist.reload(); vers.reload(); onRestored(); }} />}
    </div>
  );
}

/** "Show changes" / "Hide changes": a real bordered button with a chevron, so it reads as clickable. */
export function RevealButton({ open, onClick, label }: { open: boolean; onClick: () => void; label?: string }) {
  return <button className={cx('btn sm reveal', open && 'on')} aria-expanded={open} onClick={onClick}><LuChevronDown className="chev" /> {label || (open ? 'Hide changes' : 'Show changes')}</button>;
}

export { LuSave };
