import { useEffect, useMemo, useState } from 'react';
import { LuSave, LuGitCompareArrows, LuUndo2, LuTrash2, LuColumns2, LuRows3 } from 'react-icons/lu';
import { call } from '../ipc';
import { Modal, useAction, useConfirm, useLoad } from '../components/common';
import { ago, bytes, cx, dateTime } from '../format';

export interface Version { id: string; sha256: string; size: number; path: string; reason: string; note: string; at: number; isCurrent: boolean }

const REASON: Record<string, string> = { baseline: 'first seen', auto: 'changed', manual: 'saved by you', 'before-restore': 'before a restore' };

/** Version list shown in a file's History panel. */
export function VersionsPanel({ fileId, fileName, present, refreshKey, onRestored }: { fileId: string; fileName: string; present: boolean; refreshKey?: string; onRestored: () => void }) {
  const versions = useLoad(() => call<Version[]>('versions.list', { fileId }), [fileId, refreshKey]);
  const [note, setNote] = useState('');
  const [pick, setPick] = useState<string[]>([]);
  const [compare, setCompare] = useState<{ old: string; new: string } | null>(null);
  const act = useAction();
  const confirm = useConfirm();
  const list = versions.data || [];

  const toggle = (id: string) => setPick((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p.slice(-1), id]));
  const restore = async (v: Version) => {
    if (await confirm({ title: 'Restore this version?', confirm: 'Restore', body: <>
      <span className="mono">{fileName}</span> on disk will be replaced with the version from <b>{dateTime(v.at)}</b>. The current content is saved as a version first, so you can undo this. Close the file in other programs before restoring.
    </> })) {
      const r = await act(() => call('versions.restore', { id: v.id }), 'Version restored');
      if (r) { versions.reload(); onRestored(); }
    }
  };
  const ordered = (a: string, b: string) => {
    const ia = list.findIndex((x) => x.id === a), ib = list.findIndex((x) => x.id === b);
    return ia > ib ? { old: a, new: b } : { old: b, new: a }; // list is newest first
  };

  return (
    <div>
      {present && (
        <div className="row" style={{ marginBottom: 8 }}>
          <input className="input" placeholder="Optional note, e.g. “before re-slicing with 0.2 mm layers”" value={note} onChange={(e) => setNote(e.target.value)} />
          <button className="btn sm" onClick={async () => { const r = await act(() => call('versions.save', { fileId, note }), 'Version saved'); if (r) { setNote(''); versions.reload(); } }}><LuSave /> Save version now</button>
        </div>
      )}
      {list.length === 0 ? <div className="small muted">No stored versions. Versions are kept automatically for code/text and this project type's key files (change this under Edit → Files & versions), or save one now.</div> : <>
        <div className="small dim" style={{ marginBottom: 4 }}>Tick two versions to compare them, or compare one with the current file.</div>
        {list.map((v) => (
          <div key={v.id} className={cx('verrow', pick.includes(v.id) && 'sel')}>
            <input type="checkbox" checked={pick.includes(v.id)} onChange={() => toggle(v.id)} aria-label="Select for comparison" />
            <div className="grow" style={{ minWidth: 0 }}>
              <div className="small"><b title={dateTime(v.at)}>{ago(v.at)}</b> <span className="dim">· {REASON[v.reason] || v.reason} · {bytes(v.size)}</span> {v.isCurrent && <span className="badge ok">current</span>}</div>
              {(v.note || v.path) && <div className="small dim" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{v.note || v.path}</div>}
            </div>
            {!v.isCurrent && present && <button className="btn ghost sm" title="Compare with the current file" onClick={() => setCompare({ old: v.id, new: 'current' })}><LuGitCompareArrows /> vs now</button>}
            {!v.isCurrent && <button className="btn ghost sm" title="Restore this version" onClick={() => restore(v)}><LuUndo2 /></button>}
            <button className="btn ghost sm icon" title="Delete this stored version" onClick={async () => { if (await confirm({ title: 'Delete stored version?', danger: true, confirm: 'Delete', body: 'Only the stored copy is deleted. The file on disk is not touched.' })) { await act(() => call('versions.delete', { id: v.id })); versions.reload(); } }}><LuTrash2 /></button>
          </div>
        ))}
        {pick.length === 2 && <button className="btn primary sm" style={{ marginTop: 8 }} onClick={() => setCompare(ordered(pick[0], pick[1]))}><LuGitCompareArrows /> Compare selected</button>}
      </>}
      {compare && <CompareDialog fileId={fileId} title={fileName} old={compare.old} new={compare.new} onClose={() => setCompare(null)} onRestored={() => { versions.reload(); onRestored(); }} />}
    </div>
  );
}

/** Compare two sides of a file. Without explicit sides: previous version vs current. */
export function CompareDialog({ fileId, title, old, new: nw, onClose, onRestored }: { fileId: string; title: string; old?: string; new?: string; onClose: () => void; onRestored?: () => void }) {
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
  const setM = (m: 'unified' | 'split') => { setMode(m); localStorage.setItem('pv.diffmode', m); };
  const r = res.data;
  return (
    <Modal wide title={<>Changes in <span className="mono">{title}</span></>} onClose={onClose}>
      {err ? <div className="muted">{err}</div> : res.error ? <div className="alert bad">{res.error}</div> : !r ? <div className="muted">Comparing…</div> : <>
        <div className="row small" style={{ marginBottom: 10 }}>
          <span className="pill del">{r.old.label}</span>→<span className="pill add">{r.new.label}</span>
          {r.kind === 'text' && !r.diff.identical && !r.diff.tooLarge && <span className="grow"><span style={{ color: 'var(--ok)' }}>+{r.diff.added}</span> <span style={{ color: 'var(--bad)' }}>−{r.diff.removed}</span> lines</span>}
          {r.kind === 'text' && <div className="seg right"><button className={cx(mode === 'split' && 'on')} onClick={() => setM('split')}><LuColumns2 /> Side by side</button><button className={cx(mode === 'unified' && 'on')} onClick={() => setM('unified')}><LuRows3 /> Unified</button></div>}
        </div>
        {r.kind === 'text'
          ? (r.diff.identical ? <div className="muted">The two versions are identical.</div>
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
      <p className="small dim">Tip: use “Restore” on an older version to get it back, or open both in Blender/your slicer to compare visually.</p>
    </div>
  );
}

interface DLine { t: ' ' | '+' | '-'; text: string; o?: number; n?: number }

export function StructuredDiff({ hunks, mode }: { hunks: { lines: DLine[] }[]; mode: 'unified' | 'split' }) {
  const rows = useMemo(() => {
    if (mode === 'unified') return null;
    // pair deletions with following additions for side-by-side display
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
