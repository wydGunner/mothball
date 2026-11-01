import { useEffect, useMemo, useRef, useState } from 'react';
import {
  LuFolder, LuFolderOpen, LuChevronRight, LuChevronDown, LuSearch, LuExternalLink, LuFolderSearch, LuCopy, LuTriangleAlert,
  LuUpload, LuEye, LuPencil, LuLink2, LuX, LuHistory, LuGitCommitHorizontal, LuFileQuestion, LuCopyCheck, LuCircleCheck,
} from 'react-icons/lu';
import { call, isTauri, openPath, pickFiles, revealPath, type FileDetail, type FileDocs, type VFile } from '../ipc';
import { Empty, KindIcon, Markdown, SaveState, TagInput, useAction, useAutosave, useContextMenu, useLoad, useResizable, useToast } from '../components/common';
import { ago, bytes, cx, dateTime } from '../format';
import { LinkPicker, LinkList } from './Links';
import { CustomFields } from './Overview';
import { DiffView } from './Git';
import type { ProjectCtx } from './Project';

interface DirNode { path: string; name: string; files: number; children: DirNode[] }

function buildTree(dirs: { path: string; files: number }[]): DirNode {
  const root: DirNode = { path: '', name: '(all files)', files: 0, children: [] };
  const map = new Map<string, DirNode>([['', root]]);
  for (const d of [...dirs].sort((a, b) => a.path.localeCompare(b.path))) {
    if (d.path === '') { root.files = d.files; continue; }
    const parent = d.path.includes('/') ? d.path.slice(0, d.path.lastIndexOf('/')) : '';
    const node: DirNode = { path: d.path, name: d.path.split('/').pop()!, files: d.files, children: [] };
    map.set(d.path, node);
    (map.get(parent) || root).children.push(node);
  }
  return root;
}

const KINDS = ['code', 'document', 'config', 'image', 'mesh', 'model', 'gcode', 'archive', 'engine', 'audio', 'video', 'electronics', 'binary', 'other'];

export function FilesTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const [dir, setDir] = useState<string>('');
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState('');
  const [show, setShow] = useState<'all' | 'changed' | 'undocumented' | 'missing'>('all');
  const [sort, setSort] = useState('path');
  const [selected, setSelected] = useState<string | null>(ctx.focus?.type === 'file' ? ctx.focus.id : null);
  const [multi, setMulti] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set(['']));
  const tree = useResizable('filetree', 240, 160, 500);
  const listW = useResizable('filelist', 460, 280, 900);
  const act = useAction();
  const toast = useToast();
  const menu = useContextMenu();

  useEffect(() => { if (ctx.focus?.type === 'file') setSelected(ctx.focus.id); }, [ctx.focus]);

  const dirs = useLoad(() => call<{ path: string; files: number }[]>('files.dirs', { projectId: pid }), [pid, ctx.scanTick]);
  const files = useLoad(() => call<VFile[]>('files.list', {
    projectId: pid, query, kind: kind || null, dir: dir || null, sort,
    state: show === 'missing' ? 'missing' : null, changedOnly: show === 'changed', undocumented: show === 'undocumented', limit: 5000,
  }), [pid, query, kind, dir, show, sort, ctx.scanTick]);
  const root = useMemo(() => buildTree(dirs.data || []), [dirs.data]);

  // reveal the selected file's folder in the tree
  useEffect(() => {
    const f = files.data?.find((x) => x.id === selected);
    if (!f) return;
    const parts = f.path.split('/').slice(0, -1);
    setExpanded((e) => { const n = new Set(e); parts.forEach((_, i) => n.add(parts.slice(0, i + 1).join('/'))); return n; });
  }, [selected, files.data]);

  const importFiles = async () => {
    const paths = await pickFiles('Choose files to copy into this project');
    if (!paths.length) return;
    const r: any = await act(() => call('files.import', { projectId: pid, paths, destDir: dir }));
    if (r) { toast(`Imported ${r.imported.length}, skipped ${r.skipped.length}${r.skipped.length ? ` (${r.skipped.map((s: any) => s.reason).slice(0, 2).join('; ')})` : ''}`, 'success'); ctx.scan(true); }
  };

  const onRowClick = (e: React.MouseEvent, f: VFile, i: number) => {
    if (e.ctrlKey || e.metaKey) {
      setMulti((m) => { const n = new Set(m); n.has(f.id) ? n.delete(f.id) : n.add(f.id); return n; });
    } else if (e.shiftKey && selected && files.data) {
      const a = files.data.findIndex((x) => x.id === selected);
      const [s, t] = a < i ? [a, i] : [i, a];
      setMulti(new Set(files.data.slice(s, t + 1).map((x) => x.id)));
    } else {
      setMulti(new Set());
      setSelected(f.id);
    }
  };

  const fileMenu = async (e: React.MouseEvent, f: VFile) => {
    e.preventDefault();
    const ids = multi.size > 1 && multi.has(f.id) ? [...multi] : [f.id];
    let abs: string | undefined;
    try { abs = (await call<FileDetail>('files.get', { id: f.id })).absPath; } catch { /* missing project root */ }
    menu(e, [
      { label: 'Open with default app', icon: <LuExternalLink />, disabled: !isTauri || f.state === 'missing' || !abs, onClick: () => act(() => openPath(abs!)) },
      { label: 'Show in Explorer', icon: <LuFolderSearch />, disabled: !isTauri || f.state === 'missing' || !abs, onClick: () => act(() => revealPath(abs!)) },
      { label: 'Copy path', icon: <LuCopy />, onClick: () => { navigator.clipboard.writeText(abs || f.path); toast('Path copied'); } },
      { sep: true, label: '' },
      { label: ids.length > 1 ? `Mark ${ids.length} files reviewed` : 'Mark reviewed', icon: <LuCircleCheck />, onClick: async () => { await act(() => call('files.markReviewed', { ids })); files.reload(); ctx.reload(); } },
    ]);
  };

  const keyNav = (e: React.KeyboardEvent) => {
    if (!files.data?.length) return;
    const i = files.data.findIndex((x) => x.id === selected);
    if (e.key === 'ArrowDown') { e.preventDefault(); setSelected(files.data[Math.min(files.data.length - 1, i + 1)].id); }
    if (e.key === 'ArrowUp') { e.preventDefault(); setSelected(files.data[Math.max(0, i - 1)].id); }
  };

  const renderDir = (n: DirNode, depth: number): React.ReactNode => {
    const open = expanded.has(n.path);
    return (
      <div key={n.path}>
        <div className={cx('tnode', dir === n.path && 'on')} style={{ paddingLeft: 6 + depth * 12 }} onClick={() => setDir(n.path)} onDoubleClick={() => setExpanded((e) => { const x = new Set(e); open ? x.delete(n.path) : x.add(n.path); return x; })}>
          {n.children.length ? <span onClick={(e) => { e.stopPropagation(); setExpanded((ex) => { const x = new Set(ex); open ? x.delete(n.path) : x.add(n.path); return x; }); }}>{open ? <LuChevronDown /> : <LuChevronRight />}</span> : <span style={{ width: 14 }} />}
          {open && n.children.length ? <LuFolderOpen color="#e3b341" /> : <LuFolder color="#e3b341" />}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{n.name}</span><span className="count">{n.files}</span>
        </div>
        {open && n.children.map((c) => renderDir(c, depth + 1))}
      </div>
    );
  };

  return (
    <div className="split">
      <div className="pane side" style={{ width: tree.width, flex: 'none' }}>
        <div className="panehead"><h3 className="grow">Folders</h3>
          <button className="btn sm" onClick={importFiles} title="Copy files into this folder (or drag files onto the window)"><LuUpload /> Import</button>
        </div>
        <div className="tree">{renderDir(root, 0)}</div>
      </div>
      {tree.handle}
      <div className="pane" style={{ width: listW.width, flex: 'none', borderRight: '1px solid var(--line)', display: 'flex', flexDirection: 'column' }}>
        <div className="panehead" style={{ flexDirection: 'column', alignItems: 'stretch', gap: 6, background: 'var(--bg0)' }}>
          <div className="row"><LuSearch className="dim" /><input className="input" placeholder="Filter by name or path…" value={query} onChange={(e) => setQuery(e.target.value)} /></div>
          <div className="row wrap">
            <div className="seg">
              {([['all', 'All'], ['changed', 'Changed'], ['undocumented', 'No notes'], ['missing', 'Missing']] as const).map(([s, l]) => <button key={s} className={cx(show === s && 'on')} onClick={() => setShow(s)} title={s === 'undocumented' ? 'Files without a purpose or notes' : undefined}>{l}</button>)}
            </div>
            <select className="input" style={{ width: 110 }} value={kind} onChange={(e) => setKind(e.target.value)} aria-label="File type"><option value="">All types</option>{KINDS.map((k) => <option key={k}>{k}</option>)}</select>
            <select className="input" style={{ width: 100 }} value={sort} onChange={(e) => setSort(e.target.value)} aria-label="Sort"><option value="path">Path</option><option value="name">Name</option><option value="changed">Changed</option><option value="size">Size</option></select>
          </div>
          <div className="small dim">{files.data ? `${files.data.length}${files.data.length >= 5000 ? '+' : ''} files${dir ? ` in ${dir}/` : ''}` : 'Loading…'}{multi.size > 1 && ` · ${multi.size} selected`}</div>
        </div>
        <div style={{ overflow: 'auto', flex: 1 }} tabIndex={0} onKeyDown={keyNav}>
          {files.data?.map((f, i) => (
            <div key={f.id} className={cx('filerow', (selected === f.id || multi.has(f.id)) && 'sel', f.state === 'missing' && 'missing')}
              onClick={(e) => onRowClick(e, f, i)} onDoubleClick={() => f.state === 'present' && isTauri && call<FileDetail>('files.get', { id: f.id }).then((d) => openPath(d.absPath)).catch((e) => toast(e.message, 'error'))}
              onContextMenu={(e) => fileMenu(e, f)}>
              <KindIcon kind={f.kind} />
              <div style={{ minWidth: 0 }}>
                <div className="fname" title={f.path}>{f.name} {f.changedSinceReview && <span className="badge warn" title="Changed outside Project Vault since last reviewed">changed</span>} {f.state === 'missing' && <span className="badge bad">missing</span>}</div>
                <div className="fpurpose">{f.purpose || (dir ? f.path.slice(dir.length + 1) : f.path)}</div>
              </div>
              <span className="small dim" style={{ textAlign: 'right' }}>{bytes(f.size)}</span>
              <span className="small dim" style={{ textAlign: 'right' }}>{ago(f.lastChanged)}</span>
            </div>
          ))}
          {files.data && !files.data.length && <Empty icon={<LuFileQuestion />} title={ctx.project.fileCount ? 'No files match' : 'No files yet'}>
            {!ctx.project.fileCount && <p>{ctx.project.mode === 'managed' ? 'Import files or drag them onto the window.' : 'Click Rescan to index the folder.'}</p>}
          </Empty>}
        </div>
      </div>
      {listW.handle}
      <div className="pane grow">
        {selected ? <FileDetailPanel key={selected} fileId={selected} ctx={ctx} onChanged={() => { files.reload(); ctx.reload(); }} />
          : <Empty icon={<LuPencil />} title="Select a file">Pick a file to see what it does, add notes and warnings, link related scripts, notes and bugs, and browse its history.</Empty>}
      </div>
    </div>
  );
}

function FileDetailPanel({ fileId, ctx, onChanged }: { fileId: string; ctx: ProjectCtx; onChanged: () => void }) {
  const d = useLoad(() => call<FileDetail>('files.get', { id: fileId }), [fileId, ctx.scanTick]);
  const [docs, setDocs] = useState<FileDocs | null>(null);
  const [tags, setTags] = useState<string[]>([]);
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [linking, setLinking] = useState(false);
  const act = useAction();
  const toast = useToast();
  const docsRef = useRef<{ docs: FileDocs | null; tags: string[] }>({ docs: null, tags: [] });
  docsRef.current = { docs, tags };

  useEffect(() => {
    if (d.data && !docs) {
      setDocs(d.data.docs);
      setTags(d.data.tags);
      if (!d.data.docs.purpose && !d.data.docs.notes) setMode('edit');
    }
  }, [d.data, docs]);

  const save = useAutosave(async () => {
    const { docs, tags } = docsRef.current;
    if (!docs) return;
    await call('files.saveDocs', { id: fileId, docs: { ...docs, tags } });
    onChanged();
    d.reload();
  });
  const upd = (p: Partial<FileDocs>) => { setDocs((x) => ({ ...(x as FileDocs), ...p })); save.schedule(); };

  if (d.error) return <Empty icon={<LuTriangleAlert />} title="Cannot load file">{d.error}</Empty>;
  if (!d.data || !docs) return <div className="doc muted">Loading…</div>;
  const f = d.data;

  return (
    <div className="doc">
      <div className="row" style={{ alignItems: 'flex-start' }}>
        <KindIcon kind={f.kind} />
        <div className="grow" style={{ minWidth: 0 }}>
          <h2 style={{ wordBreak: 'break-all' }}>{f.name}</h2>
          <div className="small dim mono" style={{ wordBreak: 'break-all' }}>{f.path}</div>
        </div>
        <SaveState state={save.state} />
        {isTauri && f.state === 'present' && <>
          <button className="btn sm" onClick={() => act(() => openPath(f.absPath))}><LuExternalLink /> Open</button>
          <button className="btn sm icon" title="Show in Explorer" onClick={() => act(() => revealPath(f.absPath))}><LuFolderSearch /></button>
        </>}
        <button className="btn sm icon" title="Copy full path" onClick={() => { navigator.clipboard.writeText(f.absPath); toast('Path copied'); }}><LuCopy /></button>
      </div>

      {f.state === 'missing' && <div className="alert bad"><LuTriangleAlert /><div>This file is no longer on disk. Its documentation is kept; if the file returns (same path, or identical content elsewhere), it is re-attached.</div></div>}
      {f.docs.fileChangedSinceDoc && f.state === 'present' && (
        <div className="alert warn"><LuTriangleAlert /><div className="grow">The file's content changed after this documentation was last written. Check that the notes still apply.</div>
          <button className="btn sm" onClick={() => { save.schedule(); setTimeout(save.flush, 0); }}>Notes still accurate</button></div>
      )}
      {docs.warnings && mode === 'view' && <div className="alert warn"><LuTriangleAlert /><div style={{ whiteSpace: 'pre-wrap' }}>{docs.warnings}</div></div>}
      {f.duplicates.length > 0 && <div className="alert info"><LuCopyCheck /><div>Identical content: {f.duplicates.map((x) => <a key={x.id} href="#" onClick={(e) => { e.preventDefault(); ctx.openFile(x.id); }} className="mono" style={{ marginRight: 8 }}>{x.path}</a>)}</div></div>}

      <div className="field" style={{ margin: 0 }}>
        <label>Purpose — what this file does</label>
        <input className="input" value={docs.purpose} placeholder="What is this file for? e.g. Handles weapon-specific tuning and behavior" onChange={(e) => upd({ purpose: e.target.value })} onBlur={save.flush} />
      </div>

      <div>
        <div className="row" style={{ marginBottom: 6 }}>
          <h3 className="grow">Notes</h3>
          <div className="seg"><button className={cx(mode === 'view' && 'on')} onClick={() => { save.flush(); setMode('view'); }}><LuEye /> View</button><button className={cx(mode === 'edit' && 'on')} onClick={() => setMode('edit')}><LuPencil /> Edit</button></div>
        </div>
        {mode === 'edit' ? <>
          <textarea className="input code" rows={10} value={docs.notes} onChange={(e) => upd({ notes: e.target.value })} onBlur={save.flush}
            placeholder={'Markdown notes: how it works, what depends on it, gotchas, how to test it.\n\nLink with [[Note title]], [[file:path/to/other.file]] or [[task:Bug title]].'} />
          <div className="field" style={{ marginTop: 10 }}><label>Important warnings</label><textarea className="input" rows={2} value={docs.warnings} onChange={(e) => upd({ warnings: e.target.value })} onBlur={save.flush} placeholder="e.g. Never edit while the server is running" /></div>
        </> : (docs.notes ? <Markdown text={docs.notes} onInternalLink={(k, t) => resolveInternal(ctx, k, t, toast)} /> : <div className="muted small">No notes yet. <a href="#" onClick={(e) => { e.preventDefault(); setMode('edit'); }}>Write some</a>.</div>)}
      </div>

      <div className="docsec"><h3>Tags</h3><TagInput value={tags} onChange={(t) => { setTags(t); save.schedule(); }} /></div>

      <div className="docsec">
        <h3><LuLink2 /> Related files, notes & bugs <button className="btn sm right" onClick={() => setLinking(true)}>Add link</button></h3>
        <LinkList links={f.links} ctx={ctx} onRemoved={d.reload} />
      </div>

      <div className="docsec">
        <CustomFields value={docs.custom || {}} onChange={(custom) => upd({ custom })} />
      </div>

      <FilePreview f={f} />

      <div className="docsec">
        <h3>Details</h3>
        <div className="kv">
          <span>Type</span><span>{f.kind}{f.ext ? ` (.${f.ext})` : ''}</span>
          <span>Size</span><span>{bytes(f.size)}</span>
          {f.meshDimensionsMm && <><span>Mesh bounds</span><span>{f.meshDimensionsMm.join(' × ')} mm</span></>}
          <span>Modified</span><span>{dateTime(f.mtime)}</span>
          <span>First indexed</span><span>{dateTime(f.firstSeen)}</span>
          <span>SHA-256</span><span className="mono small">{f.sha256 || '—'}</span>
        </div>
      </div>

      <FileHistory f={f} ctx={ctx} />
      {linking && <LinkPicker ctx={ctx} from={{ type: 'file', id: f.id }} onClose={() => setLinking(false)} onLinked={d.reload} />}
    </div>
  );
}

export async function resolveInternal(ctx: ProjectCtx, kind: 'note' | 'file' | 'task', target: string, toast: (s: string, k?: any) => void) {
  try {
    if (kind === 'file') {
      const r = await call<VFile[]>('files.list', { projectId: ctx.project.id, query: target, limit: 50 });
      const exact = r.find((f) => f.path.toLowerCase() === target.toLowerCase()) || r.find((f) => f.name.toLowerCase() === target.toLowerCase()) || r[0];
      if (exact) return ctx.openFile(exact.id);
    } else if (kind === 'note') {
      const notes = await call<any[]>('notes.tree', { projectId: ctx.project.id });
      const n = notes.find((x) => x.title.toLowerCase() === target.toLowerCase());
      if (n) return ctx.openNote(n.id);
    } else {
      const tasks = await call<any[]>('tasks.list', { projectId: ctx.project.id });
      const t = tasks.find((x) => x.title.toLowerCase() === target.toLowerCase());
      if (t) return ctx.openTask(t.id);
    }
    toast(`No ${kind} named “${target}” in this project`, 'error');
  } catch (e: any) { toast(e.message, 'error'); }
}

function FilePreview({ f }: { f: FileDetail }) {
  const [open, setOpen] = useState(false);
  const [content, setContent] = useState<{ text?: string; img?: string; binary?: boolean; truncated?: boolean; error?: string } | null>(null);
  useEffect(() => {
    if (!open || f.state !== 'present') return;
    setContent(null);
    if (f.kind === 'image' && f.ext !== 'psd' && f.ext !== 'tga' && f.ext !== 'exr' && f.ext !== 'hdr' && f.ext !== 'tif' && f.ext !== 'tiff') {
      call('files.image', { id: f.id }).then((r: any) => setContent({ img: `data:${r.mime};base64,${r.base64}` }), (e) => setContent({ error: e.message }));
    } else {
      call('files.preview', { id: f.id }).then((r: any) => setContent(r), (e) => setContent({ error: e.message }));
    }
  }, [open, f.id, f.mtime]);
  if (f.state !== 'present') return null;
  return (
    <div className="docsec">
      <h3 style={{ cursor: 'pointer' }} onClick={() => setOpen(!open)}>{open ? <LuChevronDown /> : <LuChevronRight />} Preview</h3>
      {open && (!content ? <div className="muted small">Loading…</div>
        : content.error ? <div className="muted small">{content.error}</div>
        : content.img ? <div className="preview img"><img src={content.img} alt={f.name} /></div>
        : content.binary ? <div className="muted small">Binary file — no preview. Use “Open” to view it in its application.</div>
        : <><pre className="preview">{content.text}</pre>{content.truncated && <div className="small dim">Showing the first 256 KB.</div>}</>)}
    </div>
  );
}

function FileHistory({ f, ctx }: { f: FileDetail; ctx: ProjectCtx }) {
  const [tab, setTab] = useState<'changes' | 'git' | 'revisions'>('changes');
  const git = useLoad(() => tab === 'git' ? call<any[]>('git.log', { projectId: ctx.project.id, path: f.path, limit: 50 }) : Promise.resolve([]), [tab, f.id]);
  const revs = useLoad(() => tab === 'revisions' ? call<any[]>('files.docRevisions', { id: f.id }) : Promise.resolve([]), [tab, f.id]);
  const [diff, setDiff] = useState<{ title: string; text: string } | null>(null);
  return (
    <div className="docsec">
      <div className="row" style={{ marginBottom: 8 }}>
        <h3 className="grow"><LuHistory /> History</h3>
        <div className="seg">
          <button className={cx(tab === 'changes' && 'on')} onClick={() => setTab('changes')}>Detected changes</button>
          <button className={cx(tab === 'git' && 'on')} onClick={() => setTab('git')}>Git commits</button>
          <button className={cx(tab === 'revisions' && 'on')} onClick={() => setTab('revisions')}>Note revisions</button>
        </div>
      </div>
      <p className="small dim" style={{ marginTop: 0 }}>
        {tab === 'changes' && 'Changes Project Vault noticed by scanning. These record that the file changed — previous contents are not stored here (use Git for that).'}
        {tab === 'git' && 'Commits from the Git repository that touched this file (follows renames).'}
        {tab === 'revisions' && 'Earlier versions of this file’s documentation.'}
      </p>
      {tab === 'changes' && (f.events.length ? f.events.map((e, i) => (
        <div key={i} className="row small" style={{ padding: '3px 0' }}><span className="pill" style={{ minWidth: 72, justifyContent: 'center' }}>{e.kind}</span><span className="grow">{e.detail}</span><span className="dim" title={dateTime(e.at)}>{ago(e.at)}</span></div>
      )) : <div className="small muted">No changes since it was first indexed ({dateTime(f.firstSeen)}).</div>)}
      {tab === 'git' && (git.error ? <div className="small muted">{git.error}</div> : !git.data ? <div className="small muted">Loading…</div> : git.data.length === 0 ? <div className="small muted">No commits for this file (or the project is not a Git repository).</div> :
        git.data.map((c) => (
          <div key={c.hash} className="row small" style={{ padding: '4px 0', cursor: 'pointer' }} onClick={async () => { try { const r: any = await call('git.diff', { projectId: ctx.project.id, rev: c.hash, path: c.files.find((x: any) => x.path)?.path || f.path }); setDiff({ title: `${c.short} — ${c.subject}`, text: r.diff }); } catch (e: any) { setDiff({ title: 'Error', text: e.message }); } }}>
            <LuGitCommitHorizontal /><span className="mono dim">{c.short}</span><span className="grow">{c.subject}</span><span className="dim">{c.author} · {ago(c.time)}</span>
          </div>)))}
      {tab === 'revisions' && (!revs.data?.length ? <div className="small muted">No earlier revisions. (A revision is kept when documentation changes after 10+ minutes.)</div> :
        revs.data.map((r) => <details key={r.id} style={{ padding: '4px 0' }}><summary className="small">{dateTime(r.at)} — {r.purpose || <i>no purpose</i>}</summary><Markdown text={r.notes} /></details>))}
      {diff && <div style={{ marginTop: 8 }}><div className="row"><b className="small grow">{diff.title}</b><button className="btn ghost sm icon" onClick={() => setDiff(null)}><LuX /></button></div><DiffView text={diff.text} /></div>}
    </div>
  );
}
