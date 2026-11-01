import { useEffect, useMemo, useState } from 'react';
import { LuPlus, LuList, LuKanban, LuCalendar, LuBug, LuTrash2, LuChevronLeft, LuChevronRight, LuFlag, LuLink2, LuCircleCheck } from 'react-icons/lu';
import { call, type Task } from '../ipc';
import { Empty, Modal, TagInput, useAction, useConfirm, useLoad } from '../components/common';
import { ago, cx, dateTime } from '../format';
import { LinkList, LinkPicker } from './Links';
import type { ProjectCtx } from './Project';

const COLS = [
  { id: 'todo', label: 'To do' }, { id: 'in_progress', label: 'In progress' }, { id: 'blocked', label: 'Blocked' }, { id: 'done', label: 'Done' },
];
const PRIO = ['Low', 'Normal', 'High', 'Urgent'];

export function TasksTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const tasks = useLoad(() => call<Task[]>('tasks.list', { projectId: pid }), [pid]);
  const ms = useLoad(() => call<any[]>('milestones.list', { projectId: pid }), [pid]);
  const [view, setView] = useState<'list' | 'board' | 'calendar'>(() => (localStorage.getItem('pv.taskview') as any) || 'board');
  const [kind, setKind] = useState<'' | 'task' | 'bug'>('');
  const [showDone, setShowDone] = useState(true);
  const [open, setOpen] = useState<string | null>(ctx.focus?.type === 'task' && ctx.focus.id ? ctx.focus.id : null);
  const [creating, setCreating] = useState<null | 'task' | 'bug'>(null);
  const [milestoneDlg, setMilestoneDlg] = useState(false);
  const [msFilter, setMsFilter] = useState('');
  const act = useAction();
  useEffect(() => { if (ctx.focus?.type === 'task' && ctx.focus.id) setOpen(ctx.focus.id); }, [ctx.focus]);

  const list = useMemo(() => (tasks.data || []).filter((t) => (!kind || t.kind === kind) && (showDone || t.status !== 'done') && (!msFilter || t.milestoneId === msFilter)), [tasks.data, kind, showDone, msFilter]);
  const setV = (v: typeof view) => { setView(v); localStorage.setItem('pv.taskview', v); };
  const move = async (id: string, status: string) => { await act(() => call('tasks.update', { id, patch: { status } })); tasks.reload(); ms.reload(); ctx.reload(); };
  const refresh = () => { tasks.reload(); ms.reload(); ctx.reload(); };

  return (
    <div className="page" style={{ maxWidth: 'none' }}>
      <div className="row wrap">
        <button className="btn primary" onClick={() => setCreating('task')}><LuPlus /> Task</button>
        <button className="btn" onClick={() => setCreating('bug')}><LuBug /> Bug report</button>
        <div className="seg"><button className={cx(kind === '' && 'on')} onClick={() => setKind('')}>All</button><button className={cx(kind === 'task' && 'on')} onClick={() => setKind('task')}>Tasks</button><button className={cx(kind === 'bug' && 'on')} onClick={() => setKind('bug')}>Bugs</button></div>
        <select className="input" style={{ width: 180 }} value={msFilter} onChange={(e) => setMsFilter(e.target.value)} aria-label="Milestone filter"><option value="">All milestones</option>{(ms.data || []).map((m) => <option key={m.id} value={m.id}>{m.title}</option>)}</select>
        <label className="row small"><input type="checkbox" checked={showDone} onChange={(e) => setShowDone(e.target.checked)} /> Show completed</label>
        <button className="btn" onClick={() => setMilestoneDlg(true)}><LuFlag /> Milestones</button>
        <div className="seg right">
          <button className={cx(view === 'list' && 'on')} onClick={() => setV('list')}><LuList /> List</button>
          <button className={cx(view === 'board' && 'on')} onClick={() => setV('board')}><LuKanban /> Board</button>
          <button className={cx(view === 'calendar' && 'on')} onClick={() => setV('calendar')}><LuCalendar /> Calendar</button>
        </div>
      </div>
      {(ms.data || []).filter((m) => !m.done).length > 0 && (
        <div className="row wrap" style={{ margin: '12px 0' }}>
          {ms.data!.filter((m) => !m.done).map((m) => (
            <div key={m.id} className="card" style={{ padding: '6px 10px', minWidth: 200 }}>
              <div className="row small"><LuFlag color="#c58bff" /><b className="grow">{m.title}</b><span className="dim">{m.dueAt ? new Date(m.dueAt).toLocaleDateString() : ''}</span></div>
              <div className="progress" style={{ marginTop: 6 }}><div style={{ width: `${m.total ? (100 * m.completed) / m.total : 0}%`, background: '#c58bff' }} /></div>
              <div className="small dim">{m.completed}/{m.total} done</div>
            </div>
          ))}
        </div>
      )}
      <div style={{ marginTop: 14 }}>
        {tasks.data && !tasks.data.length ? <Empty icon={<LuCircleCheck />} title="No tasks yet">Track what needs doing next, and file bugs with reproduction steps.</Empty>
          : view === 'board' ? <Board list={list} onOpen={setOpen} onMove={move} />
          : view === 'list' ? <ListView list={list} onOpen={setOpen} onToggle={(t) => move(t.id, t.status === 'done' ? 'todo' : 'done')} milestones={ms.data || []} />
          : <CalendarView list={list} onOpen={setOpen} />}
      </div>
      {creating && <CreateTask kind={creating} pid={pid} onClose={() => setCreating(null)} onCreated={(t) => { setCreating(null); refresh(); setOpen(t.id); }} />}
      {open && <TaskDialog id={open} ctx={ctx} milestones={ms.data || []} onClose={() => setOpen(null)} onChanged={refresh} />}
      {milestoneDlg && <Milestones pid={pid} list={ms.data || []} onClose={() => setMilestoneDlg(false)} onChanged={refresh} />}
    </div>
  );
}

function TaskCard({ t, onOpen }: { t: Task; onOpen: (id: string) => void }) {
  const done = t.checklist.filter((c) => c.done).length;
  return (
    <div className="tcard" draggable onDragStart={(e) => e.dataTransfer.setData('text/task', t.id)} onClick={() => onOpen(t.id)}>
      <div className="t">{t.kind === 'bug' && <LuBug color="var(--bad)" style={{ marginRight: 4, verticalAlign: -2 }} />}{t.title}</div>
      <div className="row small" style={{ gap: 6, flexWrap: 'wrap' }}>
        <span className={`prio p${t.priority}`}>{PRIO[t.priority]}</span>
        {t.dueAt && <span className={cx('dim', t.status !== 'done' && t.dueAt < Date.now() && 'bad')} style={t.status !== 'done' && t.dueAt < Date.now() ? { color: 'var(--bad)' } : undefined}>due {new Date(t.dueAt).toLocaleDateString()}</span>}
        {t.checklist.length > 0 && <span className="dim">☑ {done}/{t.checklist.length}</span>}
        {t.tags.map((g) => <span className="tag" key={g}>{g}</span>)}
      </div>
    </div>
  );
}

function Board({ list, onOpen, onMove }: { list: Task[]; onOpen: (id: string) => void; onMove: (id: string, s: string) => void }) {
  const [over, setOver] = useState<string | null>(null);
  return (
    <div className="kanban">
      {COLS.map((c) => {
        const items = list.filter((t) => t.status === c.id);
        return (
          <div key={c.id} className={cx('kcol', over === c.id && 'over')} onDragOver={(e) => { e.preventDefault(); setOver(c.id); }} onDragLeave={() => setOver(null)}
            onDrop={(e) => { e.preventDefault(); setOver(null); const id = e.dataTransfer.getData('text/task'); if (id) onMove(id, c.id); }}>
            <h3>{c.label}<span className="badge">{items.length}</span></h3>
            {items.map((t) => <TaskCard key={t.id} t={t} onOpen={onOpen} />)}
          </div>
        );
      })}
    </div>
  );
}

function ListView({ list, onOpen, onToggle, milestones }: { list: Task[]; onOpen: (id: string) => void; onToggle: (t: Task) => void; milestones: any[] }) {
  const msName = (id?: string) => milestones.find((m) => m.id === id)?.title || '';
  return (
    <table className="list">
      <thead><tr><th style={{ width: 30 }} /><th>Title</th><th>Status</th><th>Priority</th><th>Due</th><th>Milestone</th><th>Updated</th></tr></thead>
      <tbody>{list.map((t) => (
        <tr key={t.id} className="click" onClick={() => onOpen(t.id)}>
          <td onClick={(e) => e.stopPropagation()}><input type="checkbox" checked={t.status === 'done'} onChange={() => onToggle(t)} aria-label="Done" /></td>
          <td style={t.status === 'done' ? { textDecoration: 'line-through', color: 'var(--fg3)' } : undefined}>{t.kind === 'bug' && <LuBug color="var(--bad)" />} {t.title} {t.tags.map((g) => <span key={g} className="tag">{g}</span>)}</td>
          <td className="muted">{COLS.find((c) => c.id === t.status)?.label}</td><td><span className={`prio p${t.priority}`}>{PRIO[t.priority]}</span></td>
          <td className="muted">{t.dueAt ? new Date(t.dueAt).toLocaleDateString() : ''}</td><td className="muted">{msName(t.milestoneId)}</td><td className="muted">{ago(t.updatedAt)}</td>
        </tr>))}
      </tbody>
    </table>
  );
}

function CalendarView({ list, onOpen }: { list: Task[]; onOpen: (id: string) => void }) {
  const [month, setMonth] = useState(() => { const d = new Date(); return new Date(d.getFullYear(), d.getMonth(), 1); });
  const start = new Date(month); start.setDate(1 - ((month.getDay() + 6) % 7));
  const days = Array.from({ length: 42 }, (_, i) => { const d = new Date(start); d.setDate(start.getDate() + i); return d; });
  const key = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  const byDay = new Map<string, Task[]>();
  for (const t of list) if (t.dueAt) { const k = key(new Date(t.dueAt)); byDay.set(k, [...(byDay.get(k) || []), t]); }
  const today = key(new Date());
  const undated = list.filter((t) => !t.dueAt && t.status !== 'done').length;
  return (
    <div>
      <div className="row" style={{ marginBottom: 8 }}>
        <button className="btn icon" onClick={() => setMonth(new Date(month.getFullYear(), month.getMonth() - 1, 1))} aria-label="Previous month"><LuChevronLeft /></button>
        <h2 style={{ width: 180, textAlign: 'center' }}>{month.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</h2>
        <button className="btn icon" onClick={() => setMonth(new Date(month.getFullYear(), month.getMonth() + 1, 1))} aria-label="Next month"><LuChevronRight /></button>
        <button className="btn sm" onClick={() => { const d = new Date(); setMonth(new Date(d.getFullYear(), d.getMonth(), 1)); }}>Today</button>
        {undated > 0 && <span className="small dim right">{undated} open task(s) without a due date are not shown</span>}
      </div>
      <div className="cal">
        {['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].map((d) => <div key={d} className="dow">{d}</div>)}
        {days.map((d) => (
          <div key={key(d)} className={cx(d.getMonth() !== month.getMonth() && 'other', key(d) === today && 'today')}>
            <span className="n">{d.getDate()}</span>
            {(byDay.get(key(d)) || []).map((t) => <span key={t.id} className={cx('ev', t.status === 'done' && 'done')} onClick={() => onOpen(t.id)}>{t.kind === 'bug' ? '🐞 ' : ''}{t.title}</span>)}
          </div>
        ))}
      </div>
    </div>
  );
}

const toDateInput = (ms?: number) => (ms ? new Date(ms - new Date(ms).getTimezoneOffset() * 60000).toISOString().slice(0, 10) : '');
const fromDateInput = (s: string) => (s ? new Date(s + 'T12:00:00').getTime() : null);

function CreateTask({ kind, pid, onClose, onCreated }: { kind: 'task' | 'bug'; pid: string; onClose: () => void; onCreated: (t: Task) => void }) {
  const [title, setTitle] = useState('');
  const [priority, setPriority] = useState(1);
  const [description, setDescription] = useState('');
  const act = useAction();
  const go = async () => { const t = await act(() => call<Task>('tasks.create', { projectId: pid, kind, title, priority, description })); if (t) onCreated(t); };
  return (
    <Modal title={kind === 'bug' ? 'New bug report' : 'New task'} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={!title.trim()} onClick={go}>Create</button></>}>
      <div className="field"><label>Title</label><input className="input" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && title.trim() && go()} /></div>
      <div className="field"><label>Priority</label><select className="input" value={priority} onChange={(e) => setPriority(Number(e.target.value))}>{PRIO.map((p, i) => <option key={p} value={i}>{p}</option>)}</select></div>
      <div className="field"><label>Description</label><textarea className="input" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} /></div>
      {kind === 'bug' && <p className="small dim">Reproduction steps, expected and actual behavior can be filled in after creating.</p>}
    </Modal>
  );
}

function TaskDialog({ id, ctx, milestones, onClose, onChanged }: { id: string; ctx: ProjectCtx; milestones: any[]; onClose: () => void; onChanged: () => void }) {
  const t = useLoad(() => call<Task>('tasks.get', { id }), [id]);
  const [draft, setDraft] = useState<Task | null>(null);
  const [newItem, setNewItem] = useState('');
  const [linking, setLinking] = useState(false);
  const act = useAction();
  const confirm = useConfirm();
  useEffect(() => { if (t.data) setDraft(t.data); }, [t.data]);
  if (t.error) return <Modal title="Task" onClose={onClose}>{t.error}</Modal>;
  if (!draft) return null;
  const set = (p: Partial<Task>) => setDraft({ ...draft, ...p });
  const save = async (close = true) => {
    const r = await act(() => call('tasks.update', { id, patch: {
      title: draft.title, description: draft.description, status: draft.status, priority: draft.priority, dueAt: draft.dueAt ?? null,
      milestoneId: draft.milestoneId || null, checklist: draft.checklist, reproSteps: draft.reproSteps, expected: draft.expected,
      actual: draft.actual, resolution: draft.resolution, tags: draft.tags, kind: draft.kind,
    } }));
    if (r) { onChanged(); if (close) onClose(); }
  };
  return (
    <Modal wide title={<span className="row">{draft.kind === 'bug' ? <LuBug color="var(--bad)" /> : <LuCircleCheck />} {draft.kind === 'bug' ? 'Bug report' : 'Task'}</span>} onClose={onClose}
      footer={<>
        {draft.status !== 'done' && t.data?.status !== 'done' && <button className="btn danger" style={{ marginRight: 'auto' }} onClick={async () => { if (await confirm({ title: 'Delete this task?', danger: true, confirm: 'Delete', body: 'Open tasks can be deleted. Completed tasks are kept in the project history.' })) { await act(() => call('tasks.delete', { id })); onChanged(); onClose(); } }}><LuTrash2 /> Delete</button>}
        <span className="small dim" style={{ marginRight: 'auto' }}>Created {dateTime(draft.createdAt)}{draft.completedAt ? ` · completed ${dateTime(draft.completedAt)}` : ''}</span>
        <button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" onClick={() => save()}>Save</button>
      </>}>
      <div className="field"><input className="input" style={{ fontSize: 16, fontWeight: 600 }} value={draft.title} onChange={(e) => set({ title: e.target.value })} aria-label="Title" /></div>
      <div className="grid3">
        <div className="field"><label>Status</label><select className="input" value={draft.status} onChange={(e) => set({ status: e.target.value })}>{COLS.map((c) => <option key={c.id} value={c.id}>{c.label}</option>)}</select></div>
        <div className="field"><label>Priority</label><select className="input" value={draft.priority} onChange={(e) => set({ priority: Number(e.target.value) })}>{PRIO.map((p, i) => <option key={p} value={i}>{p}</option>)}</select></div>
        <div className="field"><label>Due date</label><input type="date" className="input" value={toDateInput(draft.dueAt)} onChange={(e) => set({ dueAt: fromDateInput(e.target.value) ?? undefined })} /></div>
        <div className="field"><label>Milestone</label><select className="input" value={draft.milestoneId || ''} onChange={(e) => set({ milestoneId: e.target.value || undefined })}><option value="">None</option>{milestones.map((m) => <option key={m.id} value={m.id}>{m.title}</option>)}</select></div>
        <div className="field"><label>Type</label><select className="input" value={draft.kind} onChange={(e) => set({ kind: e.target.value as any })}><option value="task">Task</option><option value="bug">Bug</option></select></div>
      </div>
      <div className="field"><label>Tags</label><TagInput value={draft.tags} onChange={(tags) => set({ tags })} /></div>
      <div className="field"><label>Description / notes</label><textarea className="input" rows={4} value={draft.description} onChange={(e) => set({ description: e.target.value })} /></div>
      {draft.kind === 'bug' && <>
        <div className="field"><label>Steps to reproduce</label><textarea className="input" rows={3} value={draft.reproSteps} onChange={(e) => set({ reproSteps: e.target.value })} placeholder={'1. Start dedicated server\n2. …'} /></div>
        <div className="grid2">
          <div className="field"><label>Expected behavior</label><textarea className="input" rows={2} value={draft.expected} onChange={(e) => set({ expected: e.target.value })} /></div>
          <div className="field"><label>Actual behavior</label><textarea className="input" rows={2} value={draft.actual} onChange={(e) => set({ actual: e.target.value })} /></div>
        </div>
        <div className="field"><label>Resolution notes</label><textarea className="input" rows={2} value={draft.resolution} onChange={(e) => set({ resolution: e.target.value })} /></div>
      </>}
      <div className="field"><label>Checklist</label>
        {draft.checklist.map((c, i) => (
          <div key={i} className="row" style={{ padding: '2px 0' }}>
            <input type="checkbox" checked={c.done} onChange={() => set({ checklist: draft.checklist.map((x, j) => (j === i ? { ...x, done: !x.done } : x)) })} />
            <input className="input" style={{ border: 0, background: 'transparent', textDecoration: c.done ? 'line-through' : undefined }} value={c.text} onChange={(e) => set({ checklist: draft.checklist.map((x, j) => (j === i ? { ...x, text: e.target.value } : x)) })} />
            <button className="btn ghost sm icon" onClick={() => set({ checklist: draft.checklist.filter((_, j) => j !== i) })} aria-label="Remove item"><LuTrash2 /></button>
          </div>
        ))}
        <input className="input" placeholder="Add checklist item and press Enter" value={newItem} onChange={(e) => setNewItem(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && newItem.trim()) { set({ checklist: [...draft.checklist, { text: newItem.trim(), done: false }] }); setNewItem(''); } }} />
      </div>
      <div className="field"><label className="row">Related files & notes <button className="btn sm right" onClick={async () => { await save(false); setLinking(true); }}><LuLink2 /> Link</button></label>
        <LinkList links={t.data?.links || []} ctx={{ ...ctx, openFile: (f) => { onClose(); ctx.openFile(f); }, openNote: (n) => { onClose(); ctx.openNote(n); } }} onRemoved={t.reload} />
      </div>
      {linking && <LinkPicker ctx={ctx} from={{ type: 'task', id }} onClose={() => setLinking(false)} onLinked={t.reload} />}
    </Modal>
  );
}

function Milestones({ pid, list, onClose, onChanged }: { pid: string; list: any[]; onClose: () => void; onChanged: () => void }) {
  const [title, setTitle] = useState('');
  const [due, setDue] = useState('');
  const act = useAction();
  return (
    <Modal title="Milestones" onClose={onClose}>
      {list.map((m) => (
        <div key={m.id} className="row" style={{ padding: '5px 0', borderBottom: '1px solid var(--line)' }}>
          <input type="checkbox" checked={m.done} onChange={async () => { await act(() => call('milestones.save', { projectId: pid, id: m.id, done: !m.done, dueAt: m.dueAt })); onChanged(); }} aria-label="Reached" />
          <span className="grow" style={m.done ? { textDecoration: 'line-through', color: 'var(--fg3)' } : undefined}>{m.title}</span>
          <span className="small dim">{m.completed}/{m.total} tasks</span><span className="small dim">{m.dueAt ? new Date(m.dueAt).toLocaleDateString() : ''}</span>
        </div>
      ))}
      <div className="row" style={{ marginTop: 12 }}>
        <input className="input" placeholder="New milestone, e.g. v2.0 release" value={title} onChange={(e) => setTitle(e.target.value)} />
        <input className="input" type="date" style={{ width: 150 }} value={due} onChange={(e) => setDue(e.target.value)} />
        <button className="btn primary" disabled={!title.trim()} onClick={async () => { await act(() => call('milestones.save', { projectId: pid, title, dueAt: fromDateInput(due) })); setTitle(''); setDue(''); onChanged(); }}><LuPlus /> Add</button>
      </div>
    </Modal>
  );
}
