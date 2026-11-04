import { useState } from 'react';
import { LuChevronRight, LuPlus, LuTrash2, LuUndo2 } from 'react-icons/lu';
import { call } from '../ipc';
import { Empty, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { bytes, cx, day } from '../format';
import type { ProjectCtx } from './Project';
import { CompareDialog, StructuredDiff } from './Versions';

interface Change { change: 'added' | 'modified' | 'moved' | 'missing' | 'restored'; fileId: string; path: string; fromPath?: string; from?: string; to?: string }
interface EntryData { changes?: Change[]; total?: number; fileId?: string; path?: string; from?: string; to?: string; saves?: number }
interface Entry { id: string; kind: string; summary: string; detail: string; refType?: string; refId?: string; manual: boolean; at: number; data?: EntryData | null }

const CHANGE_LABEL: Record<Change['change'], string> = { added: 'Added', modified: 'Changed', moved: 'Moved', missing: 'Missing', restored: 'Back' };
const expandable = (e: Entry) => !!(e.data && ((e.data.changes && e.data.changes.length) || (e.data.from && e.data.to)));

export function TimelineTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const [limit, setLimit] = useState(150);
  const tl = useLoad(() => call<Entry[]>('timeline.list', { projectId: pid, limit }), [pid, limit, ctx.scanTick]);
  const [summary, setSummary] = useState('');
  const [detail, setDetail] = useState('');
  const [kinds, setKinds] = useState<'all' | 'journal' | 'files' | 'work'>('all');
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  const batches = useLoad(() => call<{ id: string; destDir: string; files: { path: string }[]; at: number; undoneAt?: number }[]>('files.importBatches', { projectId: pid }), [pid, ctx.scanTick, tl.data]);
  const undoImport = async (id: string) => {
    const b = batches.data?.find((x) => x.id === id);
    if (!b) return;
    if (!(await confirm({
      title: `Undo this import of ${b.files.length} file(s)?`, confirm: 'Undo import', danger: true,
      body: <>
        <p style={{ marginTop: 0 }}>The copies this import created in <span className="mono">/{b.destDir.replace(/^\/+/, '')}</span> are moved to the Recycle Bin — but only those that are still exactly as imported. Files you changed since, and any other files, are left alone. The original source files were never touched.</p>
        <ul className="small mono" style={{ maxHeight: 180, overflow: 'auto' }}>{b.files.slice(0, 200).map((f) => <li key={f.path}>{f.path}</li>)}</ul>
      </>,
    }))) return;
    const r: any = await act(() => call('files.undoImport', { id }));
    if (r) {
      toast(`Import undone: ${r.removed.length} removed${r.kept.length ? `, ${r.kept.length} kept (${r.kept.map((k: any) => `${k.path}: ${k.reason}`).slice(0, 3).join('; ')})` : ''}`, r.kept.length ? 'info' : 'success');
      ctx.scan(true); batches.reload();
    }
  };
  const add = async () => { await act(() => call('timeline.journal', { projectId: pid, summary, detail })); setSummary(''); setDetail(''); tl.reload(); };
  const filtered = (tl.data || []).filter((e) => kinds === 'all' || (kinds === 'journal' ? e.manual : kinds === 'files' ? ['files', 'import', 'file-doc'].includes(e.kind) : !['files', 'import'].includes(e.kind) && !e.manual));
  const groups: [string, Entry[]][] = [];
  for (const e of filtered) {
    const d = day(e.at);
    if (!groups.length || groups[groups.length - 1][0] !== d) groups.push([d, []]);
    groups[groups.length - 1][1].push(e);
  }
  const [openIds, setOpenIds] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setOpenIds((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const [compare, setCompare] = useState<null | { fileId: string; title: string; old: string; new: string }>(null);
  const open = (e: Entry) => {
    if (e.refType === 'note' && e.refId) ctx.openNote(e.refId);
    else if (e.refType === 'task' && e.refId) ctx.openTask(e.refId);
    else if (e.refType === 'file' && e.refId) ctx.openFile(e.refId);
  };
  return (
    <div className="page narrow" style={{ maxWidth: 1100 }}>
      <div className="card pad" style={{ marginBottom: 16 }}>
        <h2 style={{ marginBottom: 8 }}>Journal entry</h2>
        <input className="input" placeholder="What did you do / decide / learn?" value={summary} onChange={(e) => setSummary(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && summary.trim() && add()} />
        <textarea className="input" rows={2} style={{ marginTop: 6 }} placeholder="Details (optional)" value={detail} onChange={(e) => setDetail(e.target.value)} />
        <div className="row" style={{ marginTop: 8 }}><button className="btn primary" disabled={!summary.trim()} onClick={add}><LuPlus /> Add to timeline</button>
          <div className="seg right">{(['all', 'journal', 'work', 'files'] as const).map((k) => <button key={k} className={kinds === k ? 'on' : ''} onClick={() => setKinds(k)}>{k[0].toUpperCase() + k.slice(1)}</button>)}</div></div>
      </div>
      {!filtered.length && tl.data && <Empty title="Nothing here yet" />}
      <div className="tl">
        {groups.map(([d, items]) => (
          <div key={d}>
            <div className="tlday">{d}</div>
            {items.map((e) => (
              <div key={e.id} className={`tlitem ${e.manual ? 'manual' : e.kind}`}>
                <span className="time">{new Date(e.at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}</span>
                <div className="grow" style={{ minWidth: 0 }}>
                  {expandable(e)
                    ? <button className={cx('tl-toggle', openIds.has(e.id) && 'on')} aria-expanded={openIds.has(e.id)} onClick={() => toggle(e.id)} title={openIds.has(e.id) ? 'Hide what changed' : 'Show what changed'}>
                        <LuChevronRight className="chev" /><span>{e.summary}</span>{e.data?.saves && e.data.saves > 1 ? <span className="dim small">· {e.data.saves} saves</span> : null}
                      </button>
                    : <div style={{ cursor: e.refId ? 'pointer' : undefined }} onClick={() => open(e)}>{e.manual ? <b>{e.summary}</b> : e.summary}</div>}
                  {e.detail && <div className="small muted" style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>{e.detail}</div>}
                  {openIds.has(e.id) && e.data && (
                    <div className="tl-detail">
                      {e.data.changes
                        ? <ChangeList changes={e.data.changes} total={e.data.total || e.data.changes.length} openFile={ctx.openFile} onCompare={setCompare} />
                        : <ChangeDiff fileId={e.data.fileId!} from={e.data.from!} to={e.data.to!} title={e.data.path || ''} openFile={ctx.openFile} onCompare={setCompare} />}
                    </div>
                  )}
                </div>
                {e.refType === 'import' && e.refId && (() => {
                  const b = batches.data?.find((x) => x.id === e.refId);
                  if (!b) return null;
                  return b.undoneAt ? <span className="small dim">undone</span> : <button className="btn ghost sm" onClick={() => undoImport(b.id)}><LuUndo2 /> Undo import…</button>;
                })()}
                {e.manual && <button className="btn ghost sm icon" title="Delete journal entry" onClick={async () => { await act(() => call('timeline.deleteJournal', { id: e.id })); tl.reload(); }}><LuTrash2 /></button>}
              </div>
            ))}
          </div>
        ))}
      </div>
      {compare && <CompareDialog fileId={compare.fileId} title={compare.title} old={compare.old} new={compare.new} onClose={() => setCompare(null)} />}
      {(tl.data?.length || 0) >= limit && <button className="btn" style={{ marginTop: 12 }} onClick={() => setLimit(limit + 200)}>Load older</button>}
    </div>
  );
}

export type CompareReq = { fileId: string; title: string; old: string; new: string };

/** The files one scan found changed. Changed text files expand to their diff. */
function ChangeList({ changes, total, openFile, onCompare }: { changes: Change[]; total: number; openFile: (id: string) => void; onCompare: (c: CompareReq) => void }) {
  const [shown, setShown] = useState<string | null>(changes.length === 1 && changes[0].change === 'modified' ? changes[0].fileId : null);
  return (
    <div className="tl-changes">
      {changes.map((c) => {
        const canDiff = c.change === 'modified' && !!c.from && !!c.to;
        return (
          <div key={c.fileId + c.change}>
            <div className="tl-change">
              <span className={cx('tl-kind', c.change)}>{CHANGE_LABEL[c.change] || c.change}</span>
              <button className="linklike mono" disabled={c.change === 'missing'} onClick={() => openFile(c.fileId)} title={c.change === 'missing' ? 'This file is no longer on disk' : 'Open this file'}>{c.path}</button>
              {c.fromPath && <span className="dim small mono">from {c.fromPath}</span>}
              {canDiff && <button className="btn ghost sm right" onClick={() => setShown(shown === c.fileId ? null : c.fileId)}>{shown === c.fileId ? 'Hide changes' : 'Show changes'}</button>}
            </div>
            {canDiff && shown === c.fileId && <ChangeDiff fileId={c.fileId} from={c.from!} to={c.to!} title={c.path} openFile={openFile} onCompare={onCompare} />}
          </div>
        );
      })}
      {total > changes.length && <div className="small dim">…and {total - changes.length} more</div>}
    </div>
  );
}

/** Inline diff between two stored contents of one file. */
export function ChangeDiff({ fileId, from, to, title, openFile, onCompare }: { fileId: string; from: string; to: string; title: string; openFile: (id: string) => void; onCompare: (c: CompareReq) => void }) {
  const r = useLoad(() => call<any>('versions.compare', { fileId, old: `sha:${from}`, new: `sha:${to}` }), [fileId, from, to]);
  const full = () => onCompare({ fileId, title, old: `sha:${from}`, new: `sha:${to}` });
  if (r.error) return <div className="small muted tl-diffnote">{String(r.error).replace(/^NOT_KEPT:/, '')}</div>;
  const d = r.data;
  if (!d) return <div className="small dim tl-diffnote">Loading changes…</div>;
  return (
    <div className="tl-diff">
      <div className="row small" style={{ marginBottom: 6 }}>
        {d.kind === 'text'
          ? d.diff.identical ? <span className="muted">No text changes (only line endings or encoding).</span>
            : d.diff.tooLarge ? <span className="muted">Changed in too many places to list line by line.</span>
            : <span><span className="tl-add">+{d.diff.added}</span> <span className="tl-del">−{d.diff.removed}</span> lines</span>
          : <span className="muted">Binary file · {bytes(d.old.size)} → {bytes(d.new.size)}</span>}
        <span className="right row" style={{ gap: 4 }}>
          <button className="btn ghost sm" onClick={full}>Side by side…</button>
          <button className="btn ghost sm" onClick={() => openFile(fileId)}>Open file</button>
        </span>
      </div>
      {d.kind === 'text' && !d.diff.identical && !d.diff.tooLarge && <StructuredDiff hunks={d.diff.hunks} mode="unified" />}
    </div>
  );
}
