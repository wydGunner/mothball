import { useState } from 'react';
import { LuPlus, LuTrash2, LuUndo2 } from 'react-icons/lu';
import { call } from '../ipc';
import { Empty, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { day } from '../format';
import type { ProjectCtx } from './Project';

interface Entry { id: string; kind: string; summary: string; detail: string; refType?: string; refId?: string; manual: boolean; at: number }

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
  const open = (e: Entry) => {
    if (e.refType === 'note' && e.refId) ctx.openNote(e.refId);
    else if (e.refType === 'task' && e.refId) ctx.openTask(e.refId);
    else if (e.refType === 'file' && e.refId) ctx.openFile(e.refId);
  };
  return (
    <div className="page" style={{ maxWidth: 900 }}>
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
                <div className="grow">
                  <div style={{ cursor: e.refId ? 'pointer' : undefined }} onClick={() => open(e)}>{e.manual ? <b>{e.summary}</b> : e.summary}</div>
                  {e.detail && <div className="small muted" style={{ whiteSpace: 'pre-wrap', wordBreak: 'break-all' }}>{e.detail}</div>}
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
      {(tl.data?.length || 0) >= limit && <button className="btn" style={{ marginTop: 12 }} onClick={() => setLimit(limit + 200)}>Load older</button>}
    </div>
  );
}
