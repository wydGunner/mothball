import { useEffect, useState } from 'react';
import { LuFile, LuNotebookPen, LuListTodo, LuBug, LuX } from 'react-icons/lu';
import { call, type LinkRef } from '../ipc';
import { Modal, useAction } from '../components/common';
import { cx } from '../format';
import type { ProjectCtx } from './Project';

const ICON = { file: <LuFile />, note: <LuNotebookPen />, task: <LuListTodo /> };

export function LinkList({ links, ctx, onRemoved }: { links: LinkRef[]; ctx: ProjectCtx; onRemoved: () => void }) {
  const act = useAction();
  if (!links.length) return <div className="small muted">Nothing linked yet.</div>;
  const open = (l: LinkRef) => (l.type === 'file' ? ctx.openFile(l.id) : l.type === 'note' ? ctx.openNote(l.id) : ctx.openTask(l.id));
  return (
    <div className="col" style={{ gap: 2 }}>
      {links.map((l) => (
        <div key={l.linkId} className="row small" style={{ padding: '3px 0' }}>
          {l.type === 'task' && l.state.startsWith('bug') ? <LuBug color="var(--bad)" /> : ICON[l.type]}
          <a href="#" className={cx('grow', l.type === 'file' && 'mono')} onClick={(e) => { e.preventDefault(); open(l); }} style={l.state === 'missing' || l.state.endsWith(':done') ? { textDecoration: 'line-through', color: 'var(--fg3)' } : undefined}>{l.title}</a>
          {l.label && <span className="dim">{l.label}</span>}
          {l.type === 'task' && <span className="dim">{l.state.split(':')[1]?.replace('_', ' ')}</span>}
          <button className="btn ghost sm icon" title="Remove link" onClick={async () => { await act(() => call('links.remove', { linkId: l.linkId })); onRemoved(); }}><LuX /></button>
        </div>
      ))}
    </div>
  );
}

export function LinkPicker({ ctx, from, onClose, onLinked }: { ctx: ProjectCtx; from: { type: 'file' | 'note' | 'task'; id: string }; onClose: () => void; onLinked: () => void }) {
  const [type, setType] = useState<'file' | 'note' | 'task'>('file');
  const [q, setQ] = useState('');
  const [label, setLabel] = useState('');
  const [items, setItems] = useState<{ id: string; title: string; sub?: string }[]>([]);
  const act = useAction();
  useEffect(() => {
    const pid = ctx.project.id;
    const t = setTimeout(async () => {
      try {
        if (type === 'file') {
          const r = await call<any[]>('files.list', { projectId: pid, query: q, limit: 60 });
          setItems(r.filter((f) => f.id !== from.id).map((f) => ({ id: f.id, title: f.path, sub: f.purpose })));
        } else if (type === 'note') {
          const r = await call<any[]>('notes.tree', { projectId: pid });
          setItems(r.filter((n) => n.id !== from.id && n.title.toLowerCase().includes(q.toLowerCase())).map((n) => ({ id: n.id, title: n.title, sub: n.category })));
        } else {
          const r = await call<any[]>('tasks.list', { projectId: pid });
          setItems(r.filter((x) => x.id !== from.id && x.title.toLowerCase().includes(q.toLowerCase())).map((x) => ({ id: x.id, title: (x.kind === 'bug' ? '🐞 ' : '') + x.title, sub: x.status })));
        }
      } catch { setItems([]); }
    }, 120);
    return () => clearTimeout(t);
  }, [type, q, ctx.project.id, from.id]);
  const link = async (id: string) => {
    const ok = await act(() => call('links.add', { aType: from.type, aId: from.id, bType: type, bId: id, label }));
    if (ok) { onLinked(); onClose(); }
  };
  return (
    <Modal title="Link to…" onClose={onClose} wide>
      <div className="row" style={{ marginBottom: 10 }}>
        <div className="seg">
          <button className={cx(type === 'file' && 'on')} onClick={() => setType('file')}><LuFile /> File</button>
          <button className={cx(type === 'note' && 'on')} onClick={() => setType('note')}><LuNotebookPen /> Note</button>
          <button className={cx(type === 'task' && 'on')} onClick={() => setType('task')}><LuListTodo /> Task / bug</button>
        </div>
        <input className="input" autoFocus placeholder="Search…" value={q} onChange={(e) => setQ(e.target.value)} />
        <input className="input" style={{ width: 200 }} placeholder="Relation (optional), e.g. depends on" value={label} onChange={(e) => setLabel(e.target.value)} />
      </div>
      <div style={{ maxHeight: 380, overflow: 'auto' }}>
        {items.map((it) => (
          <div key={it.id} className="filerow" style={{ gridTemplateColumns: '20px 1fr' }} onClick={() => link(it.id)}>
            {ICON[type]}<div style={{ minWidth: 0 }}><div className={cx('fname', type === 'file' && 'mono')}>{it.title}</div>{it.sub && <div className="fpurpose">{it.sub}</div>}</div>
          </div>
        ))}
        {!items.length && <div className="muted small" style={{ padding: 10 }}>Nothing found.</div>}
      </div>
    </Modal>
  );
}
