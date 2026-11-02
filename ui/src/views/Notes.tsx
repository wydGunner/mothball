import { useEffect, useMemo, useRef, useState } from 'react';
import { LuPlus, LuChevronRight, LuChevronDown, LuNotebookPen, LuTrash2, LuEye, LuPencil, LuColumns2, LuHistory, LuLink2, LuFilePlus, LuArchiveRestore, LuCornerLeftUp } from 'react-icons/lu';
import { call, type Note } from '../ipc';
import { Empty, Markdown, Modal, SaveState, useAction, useAutosave, useConfirm, useContextMenu, useLoad, useResizable, useToast } from '../components/common';
import { ago, cx, dateTime } from '../format';
import { LinkList, LinkPicker } from './Links';
import { resolveInternal } from './Files';
import type { ProjectCtx } from './Project';

const TEMPLATE_SETS: Record<string, string[]> = {
  software: ['Architecture', 'Setup & installation', 'Configuration', 'Commands', 'Known bugs', 'Development ideas', 'Release notes'],
  game: ['Game design', 'Architecture', 'Scenes & scripts', 'Build & playtesting', 'Known bugs', 'Ideas'],
  '3d-printing': ['Print settings', 'Assembly', 'Finishing', 'Revisions', 'Ideas'],
  electronics: ['Schematic notes', 'Bill of materials', 'Firmware', 'Wiring', 'Troubleshooting'],
  addon: ['Features', 'Slash commands', 'Saved variables', 'Release checklist', 'Known bugs', 'Ideas'],
  video: ['Edit notes', 'Footage & sources', 'Music & licensing', 'Export settings', 'Revisions'],
  motion: ['Concept', 'Plugins & presets', 'Render settings', 'Revisions', 'Client feedback'],
  design: ['Brief', 'Fonts & colours', 'Export sizes', 'Revisions', 'Feedback'],
  '3d-art': ['Concept & references', 'Modelling notes', 'Materials & textures', 'Render settings', 'Revisions'],
  cad: ['Requirements', 'Design decisions', 'Tolerances & fits', 'Manufacturing', 'Revisions'],
};

export function NotesTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const tree = useLoad(() => call<Note[]>('notes.tree', { projectId: pid }), [pid]);
  const info = useLoad(() => call<any>('app.info'), []);
  const [selected, setSelected] = useState<string | null>(ctx.focus?.type === 'note' ? ctx.focus.id : null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [trash, setTrash] = useState(false);
  const side = useResizable('notetree', 260, 180, 520);
  const act = useAction();
  const confirm = useConfirm();
  const menu = useContextMenu();
  const drag = useRef<string | null>(null);

  useEffect(() => { if (ctx.focus?.type === 'note') setSelected(ctx.focus.id); }, [ctx.focus]);
  useEffect(() => { if (!selected && tree.data?.length) setSelected(tree.data[0].id); }, [tree.data, selected]);

  const children = useMemo(() => {
    const m = new Map<string, Note[]>();
    for (const n of tree.data || []) { const k = n.parentId || ''; m.set(k, [...(m.get(k) || []), n]); }
    return m;
  }, [tree.data]);

  const create = async (parentId?: string, title = 'Untitled', category = 'General') => {
    const n = await act(() => call<Note>('notes.create', { projectId: pid, parentId: parentId || null, title, category }));
    if (n) { tree.reload(); setSelected(n.id); }
    return n;
  };
  const createSet = async () => {
    const set = TEMPLATE_SETS[ctx.project.category] || ['Overview', 'Research', 'Ideas', 'Troubleshooting'];
    const root = await create(undefined, ctx.project.name, 'General');
    if (!root) return;
    for (const t of set) await act(() => call('notes.create', { projectId: pid, parentId: root.id, title: t, category: t.includes('bug') ? 'Known Bugs' : 'General' }));
    tree.reload();
  };
  const del = async (n: Note) => {
    if (await confirm({ title: `Move “${n.title}” to trash?`, confirm: 'Move to trash', danger: true, body: 'Sub-notes move up one level. You can restore it from the trash.' })) {
      await act(() => call('notes.delete', { id: n.id }));
      if (selected === n.id) setSelected(null);
      tree.reload();
    }
  };
  const move = async (id: string, parentId: string | null) => {
    if (id === parentId) return;
    await act(() => call('notes.save', { id, patch: { parentId } }));
    tree.reload();
  };

  const render = (n: Note, depth: number): React.ReactNode => {
    const kids = children.get(n.id) || [];
    const open = !collapsed.has(n.id);
    return (
      <div key={n.id}>
        <div className={cx('tnode', selected === n.id && 'on')} style={{ paddingLeft: 6 + depth * 14 }} onClick={() => setSelected(n.id)}
          draggable onDragStart={() => { drag.current = n.id; }} onDragOver={(e) => e.preventDefault()} onDrop={(e) => { e.preventDefault(); e.stopPropagation(); if (drag.current) move(drag.current, n.id); }}
          onContextMenu={(e) => menu(e, [
            { label: 'Add sub-note', icon: <LuPlus />, onClick: () => create(n.id) },
            { label: 'Move to top level', icon: <LuCornerLeftUp />, disabled: !n.parentId, onClick: () => move(n.id, null) },
            { sep: true, label: '' },
            { label: 'Move to trash', icon: <LuTrash2 />, danger: true, onClick: () => del(n) },
          ])}>
          {kids.length ? <span onClick={(e) => { e.stopPropagation(); setCollapsed((c) => { const x = new Set(c); open ? x.add(n.id) : x.delete(n.id); return x; }); }}>{open ? <LuChevronDown /> : <LuChevronRight />}</span> : <span style={{ width: 14 }} />}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{n.title}</span>
          {n.category !== 'General' && <span className="count">{n.category}</span>}
        </div>
        {open && kids.map((k) => render(k, depth + 1))}
      </div>
    );
  };

  return (
    <div className="split">
      <div className="pane side" style={{ width: side.width, flex: 'none', display: 'flex', flexDirection: 'column' }}
        onDragOver={(e) => e.preventDefault()} onDrop={(e) => { e.preventDefault(); if (drag.current) move(drag.current, null); }}>
        <div className="panehead"><h3 className="grow">Knowledge base</h3>
          <button className="btn sm icon" title="Trash" onClick={() => setTrash(true)}><LuTrash2 /></button>
          <button className="btn sm" onClick={() => create()}><LuPlus /> Note</button>
        </div>
        <div className="tree" style={{ flex: 1, overflow: 'auto' }}>
          {(children.get('') || []).map((n) => render(n, 0))}
          {tree.data && !tree.data.length && (
            <div className="small muted" style={{ padding: 10 }}>
              <p>No notes yet.</p>
              <button className="btn sm" onClick={createSet}><LuFilePlus /> Create a starter structure</button>
            </div>
          )}
        </div>
      </div>
      {side.handle}
      <div className="pane grow">
        {selected ? <NoteEditor key={selected} id={selected} ctx={ctx} categories={info.data?.noteCategories || []} onChanged={tree.reload} />
          : <Empty icon={<LuNotebookPen />} title="Project knowledge base">Architecture, setup guides, troubleshooting, ideas, journals — organized as a tree. Drag notes onto each other to nest them.</Empty>}
      </div>
      {trash && <Trash pid={pid} onClose={() => setTrash(false)} onRestored={(id) => { tree.reload(); setSelected(id); }} />}
    </div>
  );
}

function NoteEditor({ id, ctx, categories, onChanged }: { id: string; ctx: ProjectCtx; categories: string[]; onChanged: () => void }) {
  const note = useLoad(() => call<Note>('notes.get', { id }), [id]);
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [category, setCategory] = useState('General');
  const [mode, setMode] = useState<'edit' | 'split' | 'view'>(() => (localStorage.getItem('pv.notemode') as any) || 'split');
  const [loaded, setLoaded] = useState(false);
  const [showRevs, setShowRevs] = useState(false);
  const [linking, setLinking] = useState(false);
  const toast = useToast();
  const cur = useRef({ title, body, category });
  cur.current = { title, body, category };

  useEffect(() => { if (note.data && !loaded) { setTitle(note.data.title); setBody(note.data.body || ''); setCategory(note.data.category); setLoaded(true); } }, [note.data, loaded]);
  const save = useAutosave(async () => { await call('notes.save', { id, patch: cur.current }); onChanged(); }, 700);
  const setM = (m: typeof mode) => { setMode(m); localStorage.setItem('pv.notemode', m); };

  useEffect(() => {
    const k = (e: KeyboardEvent) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); save.flush(); } };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [save]);

  if (!loaded || !note.data) return <div className="doc muted">Loading…</div>;
  return (
    <div className="editor">
      <div className="panehead" style={{ background: 'var(--bg0)' }}>
        <select className="input" style={{ width: 160 }} value={category} onChange={(e) => { setCategory(e.target.value); save.schedule(); }} aria-label="Category">
          {[...new Set([...categories, category])].map((c) => <option key={c}>{c}</option>)}
        </select>
        <span className="small dim">Edited {ago(note.data.updatedAt)}</span>
        <SaveState state={save.state} />
        <div className="right row">
          <button className="btn sm" onClick={() => setLinking(true)}><LuLink2 /> Link</button>
          <button className="btn sm" onClick={() => setShowRevs(true)}><LuHistory /> Revisions</button>
          <div className="seg">
            <button className={cx(mode === 'edit' && 'on')} onClick={() => setM('edit')} title="Edit"><LuPencil /></button>
            <button className={cx(mode === 'split' && 'on')} onClick={() => setM('split')} title="Side by side"><LuColumns2 /></button>
            <button className={cx(mode === 'view' && 'on')} onClick={() => { save.flush(); setM('view'); }} title="Read"><LuEye /></button>
          </div>
        </div>
      </div>
      <input className="title" value={title} onChange={(e) => { setTitle(e.target.value); save.schedule(); }} onBlur={save.flush} aria-label="Title" />
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {mode !== 'view' && <textarea className="body" value={body} onChange={(e) => { setBody(e.target.value); save.schedule(); }} onBlur={save.flush} spellCheck
          placeholder={'Write in Markdown…\n\nLink other notes with [[Note title]], files with [[file:scripts/weapon_tuner.gsc]], tasks with [[task:Fix desync]].'} style={{ flex: 1, borderRight: mode === 'split' ? '1px solid var(--line)' : undefined }} />}
        {mode !== 'edit' && <div style={{ flex: 1, overflow: 'auto' }}><Markdown text={body || '_Empty note_'} onInternalLink={(k, t) => resolveInternal(ctx, k, t, toast)} /></div>}
      </div>
      {(note.data.links?.length || 0) > 0 && <div style={{ borderTop: '1px solid var(--line)', padding: '8px 20px', maxHeight: 160, overflow: 'auto' }}><LinkList links={note.data.links!} ctx={ctx} onRemoved={note.reload} /></div>}
      {linking && <LinkPicker ctx={ctx} from={{ type: 'note', id }} onClose={() => setLinking(false)} onLinked={note.reload} />}
      {showRevs && <Revisions id={id} onClose={() => setShowRevs(false)} onRestore={async (t, b) => { await save.flush(); try { await call('notes.save', { id, patch: { title: t, body: b, category, forceRevision: true } }); setTitle(t); setBody(b); onChanged(); toast('Revision restored', 'success'); } catch (e: any) { toast(e.message, 'error'); } setShowRevs(false); }} />}
    </div>
  );
}

function Revisions({ id, onClose, onRestore }: { id: string; onClose: () => void; onRestore: (title: string, body: string) => void }) {
  const revs = useLoad(() => call<any[]>('notes.revisions', { id }), [id]);
  const [sel, setSel] = useState<any>(null);
  return (
    <Modal title="Revision history" onClose={onClose} wide footer={sel && <button className="btn primary" onClick={() => onRestore(sel.title, sel.body)}>Restore this version</button>}>
      <p className="small dim" style={{ marginTop: 0 }}>A revision is kept whenever a note changes after more than 5 minutes. Restoring saves the current text as a revision first, so nothing is lost.</p>
      <div className="row" style={{ alignItems: 'flex-start' }}>
        <div style={{ width: 220, flex: 'none' }}>
          {(revs.data || []).map((r) => <div key={r.id} className={cx('tnode', sel?.id === r.id && 'on')} onClick={() => setSel(r)}>{dateTime(r.at)}</div>)}
          {revs.data && !revs.data.length && <div className="muted small">No earlier revisions yet.</div>}
        </div>
        <div className="grow" style={{ maxHeight: 420, overflow: 'auto' }}>{sel ? <><h2>{sel.title}</h2><Markdown text={sel.body} /></> : <div className="muted small">Select a revision.</div>}</div>
      </div>
    </Modal>
  );
}

function Trash({ pid, onClose, onRestored }: { pid: string; onClose: () => void; onRestored: (id: string) => void }) {
  const items = useLoad(() => call<Note[]>('notes.trash', { projectId: pid }), [pid]);
  const act = useAction();
  return (
    <Modal title="Trash" onClose={onClose}>
      {(items.data || []).map((n) => (
        <div key={n.id} className="row" style={{ padding: '4px 0' }}><span className="grow">{n.title}</span>
          <button className="btn sm" onClick={async () => { await act(() => call('notes.restore', { id: n.id })); onRestored(n.id); onClose(); }}><LuArchiveRestore /> Restore</button></div>
      ))}
      {items.data && !items.data.length && <div className="muted">Trash is empty.</div>}
    </Modal>
  );
}
