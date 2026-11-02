import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { CodeEditor } from '../components/CodeEditor';
import type { EditorHandle } from '../../editor/api';
import { ModelViewer } from '../components/ModelViewer';
import { useFileActions, TEXT_EDITABLE, type FileRef } from '../fileActions';
import {
  LuFolder, LuFolderOpen, LuChevronRight, LuChevronDown, LuSearch, LuExternalLink, LuFolderSearch, LuCopy, LuTriangleAlert,
  LuUpload, LuEye, LuFilter, LuCode, LuSave, LuArrowLeft, LuMaximize2, LuMinimize2, LuTrash2, LuArchiveRestore, LuEllipsis, LuShieldCheck, LuPencil, LuLink2, LuX, LuHistory, LuGitCommitHorizontal, LuFileQuestion, LuCopyCheck, LuCircleCheck, LuPlus, LuImage,
} from 'react-icons/lu';
import { call, isTauri, openPath, pickFiles, revealPath, type FileDetail, type FileDocs, type VFile } from '../ipc';
import { Dropdown, Empty, KindIcon, useConfirm, Markdown, SaveState, TagInput, useAction, useAutosave, useContextMenu, useLoad, useResizable, useToast } from '../components/common';
import { ago, bytes, cx, dateTime } from '../format';
import { LinkPicker, LinkList } from './Links';
import { CustomFields } from './Overview';
import { VersionsPanel, FileTimeline } from './Versions';
import { ImportDialog } from './Import';
import { SmartSaveDialog, changedLines, getLargeThreshold, getSaveMode, type SmartSaveChoice } from './SmartSave';
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

const FILE_SORTS: [string, string][] = [['path', 'Path'], ['name', 'Name'], ['changed', 'Last changed'], ['size', 'Size']];
type GroupBy = 'none' | 'folder' | 'type' | 'ext';
const GROUP_BYS: [GroupBy, string][] = [['none', 'No grouping'], ['folder', 'Folder'], ['type', 'File type'], ['ext', 'Extension']];
const KINDS = ['code', 'document', 'config', 'image', 'mesh', 'model', 'gcode', 'archive', 'engine', 'audio', 'video', 'electronics', 'binary', 'font', 'other'];

export function FilesTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const [dir, setDir] = useState<string>('');
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState('');
  const [show, setShow] = useState<'all' | 'changed' | 'undocumented' | 'missing' | 'deleted'>('all');
  const [sort, setSort] = useState('path');
  const [groupBy, setGroupByRaw] = useState<GroupBy>(() => (localStorage.getItem('pv.fileGroupBy') as GroupBy) || 'none');
  const setGroupBy = (g: GroupBy) => { setGroupByRaw(g); localStorage.setItem('pv.fileGroupBy', g); setClosedSecs([]); };
  const [closedSecs, setClosedSecs] = useState<string[]>([]);
  // Always opens on every file; the type chips narrow it down on request.
  const [group, setGroup] = useState<string>(ctx.fileGroup || 'all');
  const groups = useLoad(() => call<{ id: string; label: string; hint: string; count: number; exts: string[]; found: { ext: string; count: number }[] }[]>('files.groups', { projectId: pid }), [pid, ctx.scanTick]);
  useEffect(() => { if (ctx.fileGroup) setGroup(ctx.fileGroup); }, [ctx.fileGroup]);
  const exts = useMemo(() => {
    const gs = groups.data || [];
    const g = gs.find((x) => x.id === group);
    return g ? g.exts : null;
  }, [group, groups.data]);
  const [selected, setSelectedRaw] = useState<string | null>(ctx.focus?.type === 'file' ? ctx.focus.id : null);
  const [editing, setEditing] = useState<string | null>(null);
  const [importing, setImporting] = useState<string[] | null>(null);
  const [wide, setWide] = useState(() => localStorage.getItem('pv.editorWide') === '1');
  const toggleWide = () => setWide((w) => { localStorage.setItem('pv.editorWide', w ? '0' : '1'); return !w; });
  const full = wide && !!editing;
  const dirtyRef = useRef(false);
  const confirmLeave = () => !dirtyRef.current || window.confirm('You have unsaved changes in the editor. Discard them?');
  const setSelected = (id: string | null) => {
    if (id === selected) return;
    if (editing && !confirmLeave()) return;
    dirtyRef.current = false;
    setEditing(null);
    setSelectedRaw(id);
  };
  const [multi, setMulti] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set(['']));
  const tree = useResizable('filetree', 240, 160, 500);
  const listW = useResizable('filelist', 460, 280, 900);
  const act = useAction();
  const toast = useToast();
  const menu = useContextMenu();
  const fileActions = useFileActions();

  // Reveal/open requests (from search, links, Overview). Clears filters that could
  // hide the file, opens its folder, selects it, and scrolls it into view.
  const pendingReveal = useRef<string | null>(null);
  const [highlightV, setHighlightV] = useState<string | undefined>(undefined);
  const [gotoLine, setGotoLine] = useState<{ line: number; nonce: number } | undefined>(undefined);
  useEffect(() => {
    const fc = ctx.focus;
    if (fc?.type === 'folder') {
      if (editing && !confirmLeave()) return;
      setQuery(''); setKind(''); setGroup('all'); setShow('all'); setDir(fc.id); setSelectedRaw(null); setEditing(null); setHighlightV(undefined);
      return;
    }
    if (fc?.type !== 'file' || !fc.id) return;
    if (editing && editing !== fc.id && !confirmLeave()) return;
    dirtyRef.current = false;
    call<FileDetail>('files.get', { id: fc.id }).then((f) => {
      const parent = f.path.includes('/') ? f.path.slice(0, f.path.lastIndexOf('/')) : '';
      setQuery(''); setKind(''); setGroup('all');
      setShow(f.state === 'deleted' ? 'deleted' : f.state === 'missing' ? 'missing' : 'all');
      setDir(fc.action === 'reveal' ? parent : '');
      setSelectedRaw(f.id);
      setEditing(fc.action === 'edit' && f.state === 'present' ? f.id : null);
      if (fc.action === 'edit' && fc.line) setGotoLine({ line: fc.line, nonce: Date.now() });
      setHighlightV(fc.action === 'history' ? fc.versionId || '' : undefined);
      pendingReveal.current = f.id;
    }).catch(() => setSelectedRaw(fc.id));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctx.focus]);

  const dirs = useLoad(() => call<{ path: string; files: number }[]>('files.dirs', { projectId: pid }), [pid, ctx.scanTick]);
  const files = useLoad(() => call<VFile[]>('files.list', {
    projectId: pid, query, kind: kind || null, dir: dir || null, sort, exts,
    state: show === 'missing' ? 'missing' : show === 'deleted' ? 'deleted' : null, changedOnly: show === 'changed', undocumented: show === 'undocumented', limit: 5000,
  }), [pid, query, kind, dir, show, sort, ctx.scanTick, group, exts?.join(',')]);
  const root = useMemo(() => buildTree(dirs.data || []), [dirs.data]);
  useEffect(() => {
    const id = pendingReveal.current;
    if (!id || !files.data?.some((f) => f.id === id)) return;
    pendingReveal.current = null;
    requestAnimationFrame(() => {
      const el = document.querySelector(`[data-fid="${id}"]`) as HTMLElement | null;
      if (!el) return;
      el.scrollIntoView({ block: 'center', behavior: 'smooth' });
      el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash');
    });
  }, [files.data]);

  // reveal the selected file's folder in the tree
  useEffect(() => {
    const f = files.data?.find((x) => x.id === selected);
    if (!f) return;
    const parts = f.path.split('/').slice(0, -1);
    setExpanded((e) => { const n = new Set(e); parts.forEach((_, i) => n.add(parts.slice(0, i + 1).join('/'))); return n; });
  }, [selected, files.data]);

  // Optional grouping of the list (folder / type / extension), with collapsible headers.
  const fileSections = useMemo(() => {
    const all = files.data || [];
    if (groupBy === 'none') return [{ key: '', label: '', items: all }];
    const m = new Map<string, VFile[]>();
    const keyOf = (f: VFile) => {
      if (groupBy === 'folder') { const rel = dir ? f.path.slice(dir.length + 1) : f.path; return rel.includes('/') ? rel.slice(0, rel.lastIndexOf('/')) : ''; }
      if (groupBy === 'ext') return f.ext ? '.' + f.ext : '';
      return (groups.data || []).find((g) => g.id !== 'other' && g.exts.includes(f.ext))?.label || '';
    };
    for (const f of all) { const k = keyOf(f); m.set(k, [...(m.get(k) || []), f]); }
    let keys = [...m.keys()];
    if (groupBy === 'folder') keys.sort((a, b) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b, undefined, { sensitivity: 'base' })));
    else if (groupBy === 'ext') keys.sort((a, b) => (a === '' ? 1 : b === '' ? -1 : m.get(b)!.length - m.get(a)!.length || a.localeCompare(b)));
    else { const order = (groups.data || []).map((g) => g.label); keys.sort((a, b) => (a === '' ? 1 : b === '' ? -1 : order.indexOf(a) - order.indexOf(b))); }
    const empty = groupBy === 'folder' ? (dir ? `${dir.split('/').pop()}/ (this folder)` : 'Project root') : groupBy === 'ext' ? '(no extension)' : 'Other files';
    return keys.map((k) => ({ key: k || '\u0000', label: groupBy === 'folder' && k ? k + '/' : k || empty, items: m.get(k)! }));
  }, [files.data, groupBy, dir, groups.data]);
  const visible = useMemo(() => fileSections.flatMap((x) => (closedSecs.includes(x.key) ? [] : x.items)), [fileSections, closedSecs]);

  const importFiles = async () => {
    const gs = (groups.data || []).filter((g) => g.exts.length);
    const filters = gs.length ? [
      { name: `${ctx.project.name} files (${[...new Set(gs.flatMap((g) => g.exts))].slice(0, 8).map((e) => '.' + e).join(' ')}…)`, extensions: [...new Set(gs.flatMap((g) => g.exts))] },
      ...gs.map((g) => ({ name: g.label, extensions: g.exts })),
      { name: 'All files', extensions: ['*'] },
    ] : undefined;
    const paths = await pickFiles('Choose files to copy into this project', filters);
    if (paths.length) setImporting(paths);
  };

  const onRowClick = (e: React.MouseEvent, f: VFile, i: number) => {
    if (e.ctrlKey || e.metaKey) {
      setMulti((m) => { const n = new Set(m); n.has(f.id) ? n.delete(f.id) : n.add(f.id); return n; });
    } else if (e.shiftKey && selected && visible.length) {
      const a = visible.findIndex((x) => x.id === selected);
      const [s, t] = a < i ? [a, i] : [i, a];
      setMulti(new Set(visible.slice(s, t + 1).map((x) => x.id)));
    } else {
      setMulti(new Set());
      setSelected(f.id);
    }
  };

  const toRef = (f: VFile, abs?: string): FileRef => ({ id: f.id, name: f.name, path: f.path, absPath: abs, projectId: pid, projectName: ctx.project.name, state: f.state, editable: TEXT_EDITABLE(f.kind, f.ext, f.size) });
  const fileMenu = async (e: { clientX: number; clientY: number; preventDefault(): void }, f: VFile) => {
    e.preventDefault();
    const pos = { clientX: e.clientX, clientY: e.clientY, preventDefault() {} };
    const sel = multi.size > 1 && multi.has(f.id) ? (files.data || []).filter((x) => multi.has(x.id)) : [f];
    let abs: string | undefined;
    try { abs = (await call<FileDetail>('files.get', { id: f.id })).absPath; } catch { /* project root unreachable */ }
    menu(pos, fileActions.menuItems(toRef(f, abs), {
      selection: sel.map((x) => toRef(x)),
      onEdit: () => { setSelected(f.id); setEditing(f.id); },
      onHistory: () => { setEditing(null); setSelected(f.id); setHighlightV(''); setTimeout(() => setHighlightV(undefined), 400); },
      onChanged: () => { files.reload(); dirs.reload(); ctx.reload(); },
      extra: [
        { label: sel.length > 1 ? `Mark ${sel.length} files reviewed` : 'Mark reviewed', icon: <LuCircleCheck />, onClick: async () => { await act(() => call('files.markReviewed', { ids: sel.map((x) => x.id) })); files.reload(); ctx.reload(); } },
        ...(sel.length === 1 && f.state === 'present' && ['png', 'jpg', 'jpeg', 'webp', 'gif', '3mf'].includes(f.ext)
          ? [{ label: 'Use as the project picture', icon: <LuImage />, onClick: async () => { if (await act(() => call('projects.setThumbnail', { id: pid, thumb: { mode: 'file', fileId: f.id } }), 'Shown on the project card')) ctx.nav.refreshProjects(); } }] : []),
      ],
    }));
  };


  const keyNav = (e: React.KeyboardEvent) => {
    if (!visible.length) return;
    const list = visible;
    const i = list.findIndex((x) => x.id === selected);
    if (e.key === 'ArrowDown') { e.preventDefault(); setSelected(list[Math.min(list.length - 1, i + 1)].id); }
    if (e.key === 'ArrowUp') { e.preventDefault(); setSelected(list[Math.max(0, i - 1)].id); }
    if ((e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && i >= 0) {
      e.preventDefault();
      const r = (document.querySelector('.filerow.sel') as HTMLElement | null)?.getBoundingClientRect();
      fileMenu({ clientX: (r?.left ?? 200) + 60, clientY: r?.bottom ?? 200, preventDefault() {} }, list[i]);
    }
    if (e.key === 'Enter' && i >= 0 && canEdit(list[i]) && !editing) { e.preventDefault(); setEditing(list[i].id); }
  };

  const renderDir = (n: DirNode, depth: number): React.ReactNode => {
    const open = expanded.has(n.path);
    return (
      <div key={n.path}>
        <div className={cx('tnode', dir === n.path && 'on')} style={{ paddingLeft: 6 + depth * 12 }} onClick={() => setDir(n.path)} onDoubleClick={() => setExpanded((e) => { const x = new Set(e); open ? x.delete(n.path) : x.add(n.path); return x; })}>
          {n.children.length ? <span onClick={(e) => { e.stopPropagation(); setExpanded((ex) => { const x = new Set(ex); open ? x.delete(n.path) : x.add(n.path); return x; }); }}>{open ? <LuChevronDown /> : <LuChevronRight />}</span> : <span style={{ width: 14 }} />}
          {open && n.children.length ? <LuFolderOpen className="folder" /> : <LuFolder className="folder" />}
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{n.name}</span><span className="count">{n.files}</span>
        </div>
        {open && n.children.map((c) => renderDir(c, depth + 1))}
      </div>
    );
  };

  return (
    <div className="split">
      <div className="pane side" style={{ width: tree.width, flex: 'none', display: full ? 'none' : undefined }}>
        <div className="panehead"><h3 className="grow">Folders</h3>
          <button className="btn sm" onClick={importFiles} title="Copy files into this folder (or drag files onto the window)"><LuUpload /> Import</button>
        </div>
        <div className="tree">{renderDir(root, 0)}</div>
      </div>
      {importing && <ImportDialog projectId={pid} paths={importing} initialDir={dir} onClose={() => setImporting(null)} onDone={() => { ctx.scan(true); }} />}
      {!full && tree.handle}
      <div className="pane" style={{ width: listW.width, flex: 'none', borderRight: '1px solid var(--line)', display: full ? 'none' : 'flex', flexDirection: 'column' }}>
        <div className="panehead" style={{ flexDirection: 'column', alignItems: 'stretch', gap: 6, background: 'var(--bg0)' }}>
          {(groups.data || []).length > 0 && (
            <div className="chips">
              <button className={cx('chip', group === 'all' && 'on')} onClick={() => setGroup('all')}>All files <span>{ctx.project.fileCount}</span></button>
              {groups.data!.filter((g) => g.count > 0 || group === g.id).map((g) => <button key={g.id} className={cx('chip', group === g.id && 'on')} onClick={() => setGroup(g.id)} title={`${g.hint ? g.hint + ' — ' : ''}${(g.found || []).map((f) => `${f.ext ? '.' + f.ext : '(no extension)'} ${f.count}`).join(' · ')}`}>{g.label} <span>{g.count}</span></button>)}
            </div>
          )}
          <div className="row">
            <div className="searchinput grow" style={{ width: 'auto' }}><LuSearch /><input className="input" placeholder="Search files…" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search files" /></div>
            <Dropdown label="Filter" icon={<LuFilter />} badge={(show !== 'all' ? 1 : 0) + (kind ? 1 : 0) || undefined} align="right" sections={[
              { title: 'Show', items: ([['all', 'All files'], ['changed', 'Changed since reviewed'], ['undocumented', 'Without notes'], ['missing', 'Missing from disk'], ['deleted', 'Deleted (restorable)']] as const).map(([k, l]) => ({ key: k, label: l, checked: show === k, onSelect: () => setShow(k) })) },
              ...(!(groups.data || []).length ? [{ title: 'Type', items: KINDS.map((k) => ({ key: k, label: k, checked: kind === k, onSelect: () => setKind(kind === k ? '' : k) })) }] : []),
            ]} />
            <Dropdown label={<>Sort: {FILE_SORTS.find((x) => x[0] === sort)?.[1]}{groupBy !== 'none' && <> · by {GROUP_BYS.find((x) => x[0] === groupBy)?.[1].toLowerCase()}</>}</>} align="right" width={190} sections={[
              { title: 'Sort by', items: FILE_SORTS.map(([k, l]) => ({ key: k, label: l, checked: sort === k, onSelect: () => setSort(k) })) },
              { title: 'Group by', items: GROUP_BYS.map(([k, l]) => ({ key: 'g' + k, label: l, checked: groupBy === k, onSelect: () => setGroupBy(k) })) },
            ]} />
          </div>
          <div className="small dim">{files.data ? `${files.data.length}${files.data.length >= 5000 ? '+' : ''} ${files.data.length === 1 ? 'file' : 'files'}${dir ? ` in ${dir}/` : ''}${exts ? ' (filtered by type)' : ''}` : 'Loading…'}{multi.size > 1 && ` · ${multi.size} selected`}</div>
        </div>
        <div style={{ overflow: 'auto', flex: 1 }} tabIndex={0} onKeyDown={keyNav}>
          {fileSections.map((sec) => <Fragment key={sec.key}>
            {sec.label && <button className="filesec" aria-expanded={!closedSecs.includes(sec.key)} onClick={() => setClosedSecs((c) => (c.includes(sec.key) ? c.filter((x) => x !== sec.key) : [...c, sec.key]))}>
              {closedSecs.includes(sec.key) ? <LuChevronRight /> : <LuChevronDown />}<span className={cx('grow', groupBy !== 'type' && 'mono')}>{sec.label}</span><span className="count">{sec.items.length}</span>
            </button>}
            {!closedSecs.includes(sec.key) && sec.items.map((f) => { const i = visible.indexOf(f); return (
            <div key={f.id} data-fid={f.id} className={cx('filerow', (selected === f.id || multi.has(f.id)) && 'sel', f.state !== 'present' && 'missing')}
              onClick={(e) => onRowClick(e, f, i)} onDoubleClick={() => f.state === 'present' && isTauri && call<FileDetail>('files.get', { id: f.id }).then((d) => openPath(d.absPath)).catch((e) => toast(e.message, 'error'))}
              onContextMenu={(e) => fileMenu(e, f)}>
              <KindIcon kind={f.kind} />
              <div style={{ minWidth: 0 }}>
                <div className="fname" title={f.path}>{f.name} {f.versions > 1 && <span className="badge" title={`${f.versions} stored versions`}>v{f.versions}</span>} {f.changedSinceReview && <span className="badge warn" title="Changed outside Project Vault since last reviewed">changed</span>} {f.state === 'missing' && <span className="badge bad">missing</span>}</div>
                <div className="fpurpose">{f.purpose || (() => { const rel = dir ? f.path.slice(dir.length + 1) : f.path; return rel !== f.name && groupBy !== 'folder' ? rel : ''; })()}</div>
              </div>
              <span className="small dim" style={{ textAlign: 'right' }}>{bytes(f.size)}</span>
              <span className="small dim" style={{ textAlign: 'right' }}>{ago(f.lastChanged)}</span>
            </div>
          ); })}
          </Fragment>)}
          {files.data && !files.data.length && <Empty icon={<LuFileQuestion />} title={ctx.project.fileCount ? 'No files match' : 'No files yet'}>
            {ctx.project.fileCount > 0 && exts && <button className="btn sm" onClick={() => setGroup('all')}>Show all files</button>}
            {!ctx.project.fileCount && <p>{ctx.project.mode === 'managed' ? 'Import files or drag them onto the window.' : 'Click Rescan to index the folder.'}</p>}
          </Empty>}
        </div>
      </div>
      {!full && listW.handle}
      <div className="pane grow">
        {selected && editing === selected ? <EditorPane key={'ed' + selected} fileId={selected} ctx={ctx} gotoLine={gotoLine}
            onDirty={(d) => { dirtyRef.current = d; }} wide={full} onToggleWide={toggleWide}
            onClose={() => { if (confirmLeave()) { dirtyRef.current = false; setEditing(null); } }}
            onSaved={() => { files.reload(); ctx.reload(); }} />
          : selected ? <FileDetailPanel key={selected} fileId={selected} ctx={ctx} highlightVersion={highlightV} onEdit={() => setEditing(selected)} onChanged={() => { files.reload(); ctx.reload(); }} />
          : <Empty icon={<LuPencil />} title="Select a file">Pick a file to see what it does, add notes and warnings, link related scripts, notes and bugs, and browse its history.</Empty>}
      </div>
    </div>
  );
}

const EDITABLE_EXTS = new Set(['md', 'markdown', 'txt', 'csv', 'log', 'gcode', 'scad', 'meta', 'asmdef', 'gitignore', 'env', 'tscn', 'tres', 'kicad_sch', 'kicad_pro', 'svg', 'shader', 'hlsl', 'glsl']);
const canEdit = (f: VFile) => f.state === 'present' && f.size <= 8 * 1024 * 1024 && (f.kind === 'code' || f.kind === 'config' || EDITABLE_EXTS.has(f.ext) || (f.ext === '' && f.size < 1024 * 1024));

function FileDetailPanel({ fileId, ctx, onChanged, onEdit, highlightVersion }: { fileId: string; ctx: ProjectCtx; onChanged: () => void; onEdit: () => void; highlightVersion?: string }) {
  const menu = useContextMenu();
  const fileActions = useFileActions();
  const d = useLoad(() => call<FileDetail>('files.get', { id: fileId }), [fileId, ctx.scanTick]);
  const [docs, setDocs] = useState<FileDocs | null>(null);
  const [tags, setTags] = useState<string[]>([]);
  const [mode, setMode] = useState<'view' | 'edit'>('view');
  const [linking, setLinking] = useState(false);
  // empty sections stay out of the way until asked for
  const [show, setShow] = useState({ warn: false, tags: false, custom: false });
  const act = useAction();
  const toast = useToast();
  const docsRef = useRef<{ docs: FileDocs | null; tags: string[] }>({ docs: null, tags: [] });
  docsRef.current = { docs, tags };

  useEffect(() => {
    if (d.data && !docs) {
      setDocs(d.data.docs);
      setTags(d.data.tags);
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
          <div className="small dim mono" style={{ wordBreak: 'break-all' }}>{f.path.includes('/') ? f.path : 'in the project root'}</div>
        </div>
        <SaveState state={save.state} />
        {canEdit(f) && <button className="btn primary sm" onClick={onEdit} title="Edit this file in Project Vault"><LuCode /> Edit file</button>}
        {isTauri && f.state === 'present' && <>
          <button className="btn sm" onClick={() => act(() => openPath(f.absPath))}><LuExternalLink /> Open</button>
          <button className="btn sm icon" title="Show in Explorer" onClick={() => act(() => revealPath(f.absPath))}><LuFolderSearch /></button>
        </>}
        <button className="btn sm icon" title="More actions (rename, delete, copy path…)" onClick={(e) => {
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          menu({ clientX: r.right - 200, clientY: r.bottom + 4, preventDefault() {} }, fileActions.menuItems(
            { id: f.id, name: f.name, path: f.path, absPath: f.absPath, projectId: f.projectId, state: f.state, editable: canEdit(f) },
            { onEdit, onChanged: () => { d.reload(); onChanged(); } }));
        }}><LuEllipsis /></button>
      </div>

      {f.state === 'deleted' && <div className="alert bad"><LuTrash2 /><div className="grow">You deleted this file in Project Vault (it went to the Recycle Bin). Its notes and history are kept.</div>
        <button className="btn sm" onClick={async () => { if (await fileActions.restore({ id: f.id, name: f.name, path: f.path, projectId: f.projectId, state: f.state })) { d.reload(); onChanged(); } }}><LuArchiveRestore /> Restore</button></div>}
      {f.state === 'missing' && <div className="alert bad"><LuTriangleAlert /><div>This file is no longer on disk. Its documentation is kept; if the file returns (same path, or identical content elsewhere), it is re-attached.</div></div>}
      {f.docs.fileChangedSinceDoc && f.state === 'present' && (
        <div className="alert warn"><LuTriangleAlert /><div className="grow">The file's content changed after this documentation was last written. Check that the notes still apply.</div>
          <button className="btn sm" onClick={() => { save.schedule(); setTimeout(save.flush, 0); }}>Notes still accurate</button></div>
      )}
      {docs.warnings && !show.warn && <div className="alert warn"><LuTriangleAlert /><div className="grow" style={{ whiteSpace: 'pre-wrap' }}>{docs.warnings}</div><button className="btn ghost sm icon" title="Edit warning" onClick={() => setShow((x) => ({ ...x, warn: true }))}><LuPencil /></button></div>}
      {f.duplicates.length > 0 && <div className="alert info"><LuCopyCheck /><div>Identical content: {f.duplicates.map((x) => <a key={x.id} href="#" onClick={(e) => { e.preventDefault(); ctx.openFile(x.id); }} className="mono" style={{ marginRight: 8 }}>{x.path}</a>)}</div></div>}

      <div className="field" style={{ margin: 0 }}>
        <label>What this file does</label>
        <input className="input" value={docs.purpose} placeholder="One line, e.g. Handles weapon-specific tuning and behavior" onChange={(e) => upd({ purpose: e.target.value })} onBlur={save.flush} />
      </div>

      {(docs.notes || mode === 'edit') && (
        <div>
          <div className="row" style={{ marginBottom: 6 }}>
            <h3 className="grow">Notes</h3>
            {mode === 'edit'
              ? <button className="btn sm" onClick={() => { save.flush(); setMode('view'); }}><LuEye /> Done</button>
              : <button className="btn ghost sm" onClick={() => setMode('edit')}><LuPencil /> Edit</button>}
          </div>
          {mode === 'edit'
            ? <textarea className="input code" rows={8} autoFocus value={docs.notes} onChange={(e) => upd({ notes: e.target.value })} onBlur={save.flush}
                placeholder={'How it works, what depends on it, gotchas, how to test it (Markdown).\nLink with [[Note title]], [[file:path/to/other.file]] or [[task:Bug title]].'} />
            : <Markdown text={docs.notes} onInternalLink={(k, t) => resolveInternal(ctx, k, t, toast)} />}
        </div>
      )}

      {show.warn && (
        <div className="field" style={{ margin: 0 }}><label>Important warning</label><textarea className="input" rows={2} autoFocus={show.warn} value={docs.warnings} onChange={(e) => upd({ warnings: e.target.value })} onBlur={() => { save.flush(); if (!docs.warnings) setShow((x) => ({ ...x, warn: false })); }} placeholder="e.g. Never edit while the server is running" /></div>
      )}

      {(tags.length > 0 || show.tags) && (
        <div className="row" style={{ alignItems: 'center' }}><span className="small dim" style={{ width: 60 }}>Tags</span><div className="grow"><TagInput value={tags} onChange={(t) => { setTags(t); save.schedule(); }} /></div></div>
      )}

      {f.links.length > 0 && (
        <div>
          <div className="row" style={{ marginBottom: 6 }}><h3 className="grow"><LuLink2 /> Related</h3><button className="btn ghost sm" onClick={() => setLinking(true)}><LuPlus /> Link</button></div>
          <LinkList links={f.links} ctx={ctx} onRemoved={d.reload} />
        </div>
      )}

      {(Object.keys(docs.custom || {}).length > 0 || show.custom) && <CustomFields value={docs.custom || {}} onChange={(custom) => upd({ custom })} compact />}

      {(() => {
        const add = [
          !docs.notes && mode !== 'edit' && { label: 'Notes', on: () => setMode('edit') },
          !docs.warnings && !show.warn && { label: 'Warning', on: () => setShow((x) => ({ ...x, warn: true })) },
          !tags.length && !show.tags && { label: 'Tags', on: () => setShow((x) => ({ ...x, tags: true })) },
          { label: f.links.length ? 'Another link' : 'Link to a file, note or bug', on: () => setLinking(true) },
          !Object.keys(docs.custom || {}).length && !show.custom && { label: 'Custom field', on: () => setShow((x) => ({ ...x, custom: true })) },
        ].filter(Boolean) as { label: string; on: () => void }[];
        return add.length > 0 && <div className="addrow">{add.map((x) => <button key={x.label} className="chip" onClick={x.on}><LuPlus /> {x.label}</button>)}</div>;
      })()}

      {f.ext === 'toc' && f.state === 'present' && <TocInfo f={f} ctx={ctx} />}
      {SCRIPT_EXTS.has(f.ext) && f.state === 'present' && <ScriptOutline f={f} ctx={ctx} />}

      <FilePreview f={f} />

      <div className="docsec">
        <h3>Details</h3>
        <div className="kv">
          <span>Type</span><span>{f.kind}{f.ext ? ` (.${f.ext})` : ''} · {bytes(f.size)}</span>
          {f.meshDimensionsMm && <><span>Mesh bounds</span><span>{f.meshDimensionsMm.join(' × ')} mm</span></>}
          <span>Modified</span><span>{dateTime(f.mtime)}</span>
          <span>First indexed</span><span>{dateTime(f.firstSeen)}</span>
          <span>SHA-256</span><span className="mono small" title={f.sha256 || ''} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', wordBreak: 'normal' }}>{f.sha256 || '—'}</span>
        </div>
      </div>

      <FileHistory f={f} ctx={ctx} highlight={highlightVersion} />
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

const wowText = (s: string) => (s || '').replace(/\|c[0-9a-fA-F]{8}|\|r/g, '');
const wowFlavor = (v: string) => { const major = Number(v.split('.')[0]); return major >= 11 ? 'Retail' : major === 1 ? 'Classic Era' : 'Classic'; };

/** A WoW addon .toc: its details, the files it loads (missing ones flagged), and Lua/XML files nothing loads. */
function TocInfo({ f, ctx }: { f: FileDetail; ctx: ProjectCtx }) {
  const info = useLoad(() => call<any>('symbols.toc', { fileId: f.id }), [f.id, f.sha256]);
  const [open, setOpen] = useState(true);
  const go = (fileId: string) => ctx.nav.go({ view: 'project', id: ctx.project.id, tab: 'files', focus: { type: 'file', id: fileId, action: 'reveal', nonce: Date.now() } });
  if (info.error) return <div className="docsec"><h3>Addon</h3><div className="small muted">{info.error}</div></div>;
  const d = info.data;
  if (!d) return null;
  const fl = d.fields || {};
  const missing = d.files.filter((x: any) => x.status === 'missing');
  const rows: [string, string][] = ([
    ['Title', wowText(fl.Title)], ['Version', fl.Version], ['Game version', d.gameVersions.map((v: string) => `${v} (${wowFlavor(v)})`).join(', ')],
    ['Author', fl.Author], ['Notes', wowText(fl.Notes)], ['Saved variables', [fl.SavedVariables, fl.SavedVariablesPerCharacter && `${fl.SavedVariablesPerCharacter} (per character)`].filter(Boolean).join(', ')],
    ['Requires', fl.RequiredDeps || fl.Dependencies || fl.Dep], ['Optional', fl.OptionalDeps], ['Embedded libraries', fl['X-Embeds']],
  ] as [string, string | undefined][]).filter(([, v]) => v) as [string, string][];
  return (
    <div className="docsec">
      <h3 style={{ cursor: 'pointer' }} onClick={() => setOpen(!open)}>{open ? <LuChevronDown /> : <LuChevronRight />} Addon <span className="small dim" style={{ fontWeight: 400 }}>{d.files.length} file{d.files.length === 1 ? '' : 's'} in load order{missing.length ? ` · ${missing.length} missing` : ''}</span></h3>
      {open && <>
        {rows.length > 0 && <div className="kv" style={{ marginBottom: 10 }}>{rows.map(([k, v]) => <Fragment key={k}><span>{k}</span><span>{v}</span></Fragment>)}</div>}
        {missing.length > 0 && <div className="alert bad small" style={{ marginBottom: 8 }}><LuTriangleAlert /> {missing.length === 1 ? 'A file this .toc loads doesn’t exist' : `${missing.length} files this .toc loads don’t exist`} — the game will show a load error.</div>}
        <div className="toclist">
          {d.files.map((x: any, i: number) => (
            <div key={i} className={cx('tocrow', x.status)}>
              <span className="dim small">{i + 1}</span>
              {x.fileId ? <button className="linklike mono" onClick={() => go(x.fileId)} title={`Line ${x.line} of the .toc`}>{x.entry}</button> : <span className="mono">{x.entry}</span>}
              {x.status === 'missing' && <span className="badge bad">missing</span>}
              {x.status === 'variable' && <span className="badge" title="Contains a [variable] the game fills in, so it isn't checked">not checked</span>}
            </div>
          ))}
        </div>
        {d.notLoaded.length > 0 && <div className="small muted" style={{ marginTop: 8 }}>
          Not loaded by this .toc (or any XML it loads): {d.notLoaded.slice(0, 12).map((x: any, i: number) => <Fragment key={x.fileId}>{i > 0 && ', '}<button className="linklike mono" onClick={() => go(x.fileId)}>{x.path}</button></Fragment>)}{d.notLoaded.length > 12 ? ` and ${d.notLoaded.length - 12} more` : ''}
        </div>}
      </>}
    </div>
  );
}

/** Functions defined in a script, its #includes, and where each function is used. */
function ScriptOutline({ f, ctx }: { f: FileDetail; ctx: ProjectCtx }) {
  const syms = useLoad(() => call<any[]>('symbols.inFile', { fileId: f.id }), [f.id, f.sha256]);
  const [open, setOpen] = useState(true);
  const [q, setQ] = useState('');
  const [refs, setRefs] = useState<{ name: string; items: any[] | null } | null>(null);
  const go = (fileId: string, line?: number) => ctx.nav.go({ view: 'project', id: ctx.project.id, tab: 'files', focus: { type: 'file', id: fileId, action: line ? 'edit' : 'reveal', line, nonce: Date.now() } });
  const fns = (syms.data || []).filter((s) => s.kind === 'function' && (!q || s.name.toLowerCase().includes(q.toLowerCase())));
  const incs = (syms.data || []).filter((s) => s.kind === 'include');
  const showRefs = async (name: string) => {
    setRefs({ name, items: null });
    try { setRefs({ name, items: await call<any[]>('symbols.references', { projectId: ctx.project.id, name }) }); } catch { setRefs({ name, items: [] }); }
  };
  return (
    <div className="docsec">
      <h3 style={{ cursor: 'pointer' }} onClick={() => setOpen(!open)}>{open ? <LuChevronDown /> : <LuChevronRight />} Functions <span className="small dim" style={{ fontWeight: 400 }}>{syms.data ? `${(syms.data || []).filter((s) => s.kind === 'function').length} defined${incs.length ? ` · ${incs.length} include${incs.length === 1 ? '' : 's'}` : ''}` : ''}</span></h3>
      {open && (syms.error ? <div className="small muted">{syms.error}</div> : !syms.data ? <div className="small muted">Indexing…</div> : <>
        {incs.length > 0 && <div className="row wrap small" style={{ gap: 4, marginBottom: 8 }}>
          {incs.map((i) => i.targetId
            ? <button key={i.line} className="chip" title={`Line ${i.line} — open ${i.targetPath}`} onClick={() => go(i.targetId)}>#{i.signature.replace(/^#\w+\s*/, '')}</button>
            : <span key={i.line} className="chip" style={{ opacity: 0.6 }} title="Not in this project (engine or another mod)">#{i.signature.replace(/^#\w+\s*/, '')}</span>)}
        </div>}
        {(syms.data || []).some((s) => s.kind === 'function') ? <>
          {fns.length > 8 || q ? <div className="searchinput" style={{ width: '100%', marginBottom: 6 }}><LuSearch /><input className="input" placeholder="Filter functions…" value={q} onChange={(e) => setQ(e.target.value)} /></div> : null}
          <div className="symlist">
            {fns.map((s) => (
              <div key={s.line} className="symrow">
                <button className="symname mono" title={`Open in the editor at line ${s.line}`} onClick={() => go(f.id, s.line)}>{s.signature}</button>
                <span className="dim small">:{s.line}</span>
                <button className="btn ghost sm right" title="Where this function is called" onClick={() => showRefs(s.name)}>Uses</button>
              </div>
            ))}
            {!fns.length && <div className="small muted">No functions match.</div>}
          </div>
        </> : <div className="small muted">No function definitions in this file.</div>}
        {refs && <div className="refs">
          <div className="row small"><b className="grow">Calls to <span className="mono">{refs.name}</span></b><button className="btn ghost sm icon" onClick={() => setRefs(null)}><LuX /></button></div>
          {!refs.items ? <div className="small muted">Searching the project…</div> : !refs.items.length ? <div className="small muted">Not called anywhere in this project's scripts.</div>
            : refs.items.map((r, i) => <button key={i} className="refrow" onClick={() => go(r.fileId, r.line)}><span className="mono small">{r.path}:{r.line}</span><span className="mono small dim">{r.text}</span></button>)}
        </div>}
        <p className="small dim" style={{ marginBottom: 0 }}>In the editor, press F12 or Ctrl+click a function name to jump to where it's defined.</p>
      </>)}
    </div>
  );
}

const MODEL_EXTS = new Set(['stl', 'obj', '3mf']);

function FilePreview({ f }: { f: FileDetail }) {
  if (MODEL_EXTS.has(f.ext) && f.state === 'present') return <ModelPreview f={f} />;
  return <TextImagePreview f={f} />;
}

function ModelPreview({ f }: { f: FileDetail }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      {f.ext === '3mf' && <PrintInfo f={f} />}
      <div className="docsec">
        <h3 style={{ cursor: 'pointer' }} onClick={() => setOpen(!open)}>{open ? <LuChevronDown /> : <LuChevronRight />} 3D preview</h3>
        {open && <ModelViewer key={f.id + f.sha256} fileId={f.id} size={f.size} auto={f.size <= 40 * 1024 * 1024} />}
      </div>
    </>
  );
}

const fmtDuration = (s: number) => { const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60); return h ? `${h} h ${m} min` : `${m} min`; };

/** What the slicer stored inside a .3mf — only fields the file actually has. */
function PrintInfo({ f }: { f: FileDetail }) {
  const info = useLoad(() => call<any>('files.threemfInfo', { id: f.id }), [f.id, f.sha256]);
  if (info.error) return <div className="docsec"><h3>Print settings</h3><div className="small muted">{info.error}</div></div>;
  if (!info.data) return <div className="docsec"><h3>Print settings</h3><div className="small muted">Reading the .3mf…</div></div>;
  const d = info.data, s = d.settings || {};
  const rows: [string, string | undefined][] = [
    ['Printer', s.printer || s.printerModel], ['Process', s.process], ['Layer height', s.layerHeight && `${s.layerHeight} mm${s.firstLayerHeight ? ` (first ${s.firstLayerHeight} mm)` : ''}`],
    ['Nozzle', s.nozzle && `${s.nozzle} mm`], ['Infill', s.infill && `${s.infill}${s.infillPattern ? ` · ${s.infillPattern}` : ''}`], ['Walls', s.walls],
    ['Supports', s.supports !== undefined ? (s.supports === '1' || s.supports === 'true' ? `on${s.supportType ? ` · ${s.supportType}` : ''}` : 'off') : undefined], ['Plate', s.bed], ['Brim', s.brim],
  ];
  const known = rows.filter(([, v]) => v);
  const plates = (d.plates || []).filter((p: any) => p.printSeconds || p.weightG);
  return (
    <div className="docsec">
      <h3>Print settings <span className="small dim" style={{ fontWeight: 400 }}>from the .3mf{d.source ? ` (${d.source})` : ''}</span></h3>
      <div className="printinfo">
        {d.thumbnail && <img className="pi-thumb" src={d.thumbnail} alt="Plate preview saved by the slicer" />}
        <div className="grow" style={{ minWidth: 0 }}>
          {(d.title || d.designer) && <div style={{ marginBottom: 6 }}>{d.title && <b>{d.title}</b>}{d.designer && <span className="small dim"> by {d.designer}</span>}</div>}
          {known.length > 0 ? <div className="kv">{known.map(([k, v]) => <Fragment key={k}><span>{k}</span><span>{v}</span></Fragment>)}</div>
            : <div className="small muted">No slicer settings are stored in this file.</div>}
          {s.filaments?.length > 0 && <div style={{ marginTop: 8 }}>
            <div className="small dim" style={{ marginBottom: 4 }}>Filaments</div>
            {s.filaments.map((x: any, i: number) => <div key={i} className="row small" style={{ gap: 6 }}><span className="swatch" style={{ background: /^#[0-9a-f]{6}/i.test(x.color) ? x.color.slice(0, 7) : 'transparent' }} /><span>{x.type}</span><span className="dim">{x.name}</span></div>)}
          </div>}
          {plates.length > 0 && <div style={{ marginTop: 8 }}>
            {plates.map((p: any, i: number) => <div key={i} className="small">Plate {p.index || i + 1}: {p.printSeconds ? fmtDuration(p.printSeconds) : ''}{p.weightG ? ` · ${p.weightG.toFixed(1)} g` : ''}</div>)}
          </div>}
          {!d.sliced && <div className="small dim" style={{ marginTop: 6 }}>Not sliced yet — print time and filament use appear after slicing and saving the project.</div>}
          {d.description && <details style={{ marginTop: 6 }}><summary className="small">Model description</summary><div className="small" style={{ whiteSpace: 'pre-wrap', marginTop: 4 }}>{d.description}</div></details>}
        </div>
      </div>
    </div>
  );
}

function TextImagePreview({ f }: { f: FileDetail }) {
  const [open, setOpen] = useState(() => f.kind !== 'image' && canEdit(f) && f.size <= 64 * 1024);
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

function FileHistory({ f, ctx, highlight }: { f: FileDetail; ctx: ProjectCtx; highlight?: string }) {
  const [tab, setTab] = useState<'timeline' | 'versions' | 'changes' | 'git' | 'revisions'>(highlight ? 'versions' : 'timeline');
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (highlight === undefined) return;
    if (highlight) setTab('versions');
    setTimeout(() => box.current?.scrollIntoView({ block: 'start', behavior: 'smooth' }), 150);
  }, [highlight]);
  const git = useLoad(() => tab === 'git' ? call<any[]>('git.log', { projectId: ctx.project.id, path: f.path, limit: 50 }) : Promise.resolve([]), [tab, f.id]);
  const revs = useLoad(() => tab === 'revisions' ? call<any[]>('files.docRevisions', { id: f.id }) : Promise.resolve([]), [tab, f.id]);
  const [diff, setDiff] = useState<{ title: string; text: string } | null>(null);
  return (
    <div className="docsec" ref={box}>
      <div className="row" style={{ marginBottom: 8, flexWrap: 'wrap' }}>
        <h3 className="grow" style={{ whiteSpace: 'nowrap' }}><LuHistory /> History</h3>
        <div className="seg" style={{ flexWrap: 'wrap' }}>
          <button className={cx(tab === 'timeline' && 'on')} onClick={() => setTab('timeline')}>Timeline</button>
          <button className={cx(tab === 'versions' && 'on')} onClick={() => setTab('versions')}>Backups &amp; versions{f.versions ? ` (${f.versions})` : ''}</button>
          <button className={cx(tab === 'changes' && 'on')} onClick={() => setTab('changes')}>Activity</button>
          <button className={cx(tab === 'git' && 'on')} onClick={() => setTab('git')}>Git commits</button>
          <button className={cx(tab === 'revisions' && 'on')} onClick={() => setTab('revisions')}>Note edits</button>
        </div>
      </div>
      <p className="small dim" style={{ marginTop: 0 }}>
        {tab === 'timeline' && 'Everything known about this file, newest first. Each entry says where it came from; only entries with stored contents can be restored.'}
        {tab === 'versions' && 'Stored copies of this file. Compare any two, or roll back.'}
        {tab === 'changes' && 'When Project Vault noticed the file being added, edited, moved or deleted.'}
        {tab === 'git' && 'Commits from the Git repository that touched this file (follows renames).'}
        {tab === 'revisions' && 'Earlier versions of this file’s documentation.'}
      </p>
      {tab === 'timeline' && <FileTimeline fileId={f.id} fileName={f.name} absPath={f.absPath} path={f.path} projectId={ctx.project.id} present={f.state === 'present'} refreshKey={`${f.sha256}-${f.versions}-${ctx.scanTick}`} onRestored={() => ctx.scan(true)} />}
      {tab === 'versions' && <VersionsPanel fileId={f.id} fileName={f.name} absPath={f.absPath} present={f.state === 'present'} highlight={highlight} refreshKey={`${f.sha256}-${f.versions}-${ctx.scanTick}`} onRestored={() => ctx.scan(true)} />}
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

const SCRIPT_EXTS = new Set(['gsc', 'csc', 'gsh', 'lua']);

function EditorPane({ fileId, ctx, onClose, onSaved, onDirty, wide, onToggleWide, gotoLine }: { fileId: string; ctx: ProjectCtx; onClose: () => void; onSaved: () => void; onDirty: (d: boolean) => void; wide: boolean; onToggleWide: () => void; gotoLine?: { line: number; nonce: number } }) {
  const doc = useLoad(() => call<any>('files.readText', { id: fileId }), [fileId]);
  const info = useLoad(() => call<FileDetail>('files.get', { id: fileId }), [fileId]);
  const [text, setText] = useState<string | null>(null);
  const [sha, setSha] = useState<string>('');
  const [saved, setSaved] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const toast = useToast();
  const confirm = useConfirm();
  const latest = useRef({ text, sha, saved });
  latest.current = { text, sha, saved };
  useEffect(() => { if (doc.data && text === null) { setText(doc.data.text); setSaved(doc.data.text); setSha(doc.data.sha256); } }, [doc.data, text]);
  const dirty = text !== null && text !== saved;
  useEffect(() => { onDirty(dirty); }, [dirty, onDirty]);
  useEffect(() => {
    const warn = (e: BeforeUnloadEvent) => { if (latest.current.text !== latest.current.saved) { e.preventDefault(); e.returnValue = ''; } };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, []);

  const [smart, setSmart] = useState(false);
  const edHandle = useRef<EditorHandle | null>(null);
  // keep typing after the Smart Save dialog closes
  const refocus = () => setTimeout(() => edHandle.current?.focus(), 0);
  // jump to a line once the editor exists (from search, go-to-definition, the function list)
  const pendingLine = useRef<number | null>(null);
  useEffect(() => {
    if (!gotoLine) return;
    if (edHandle.current) edHandle.current.revealLine(gotoLine.line); else pendingLine.current = gotoLine.line;
  }, [gotoLine]);
  const menu = useContextMenu();
  const isScript = SCRIPT_EXTS.has((info.data?.ext || doc.data?.path?.split('.').pop() || '').toLowerCase());
  const openDef = (d: { fileId: string; line: number }) => {
    if (d.fileId === fileId) edHandle.current?.revealLine(d.line);
    else ctx.nav.go({ view: 'project', id: ctx.project.id, tab: 'files', focus: { type: 'file', id: d.fileId, action: 'edit', line: d.line, nonce: Date.now() } });
  };
  const goToDefinition = async (word: string, qualifier: string) => {
    let defs: any[] = [];
    try { defs = await call<any[]>('symbols.find', { projectId: ctx.project.id, name: word, qualifier }); } catch (e: any) { toast(e.message, 'error'); return; }
    if (!defs.length) { toast(`No function “${word}” is defined in this project's scripts — it may be a built-in engine function or come from another mod.`, 'info'); return; }
    const preferred = qualifier ? defs.filter((d) => { const stem = d.path.replace(/\.[^.]+$/, '').toLowerCase(); const q = qualifier.replace(/\\/g, '/').toLowerCase(); return stem === q || stem.endsWith('/' + q); }) : [];
    if (defs.length === 1 || preferred.length === 1) return openDef(preferred[0] || defs[0]);
    const box = document.querySelector('.editor-pane .panehead')?.getBoundingClientRect();
    menu({ clientX: (box?.left || 300) + 60, clientY: (box?.bottom || 120) + 4, preventDefault() {} },
      defs.slice(0, 20).map((d) => ({ label: `${d.signature} — ${d.path}:${d.line}`, onClick: () => openDef(d) })));
  };
  const hoverCache = useRef(new Map<string, string | null>());
  const hover = async (word: string, qualifier: string) => {
    const k = `${qualifier}::${word}`.toLowerCase();
    if (hoverCache.current.has(k)) return hoverCache.current.get(k)!;
    let text: string | null = null;
    try {
      const defs = await call<any[]>('symbols.find', { projectId: ctx.project.id, name: word, qualifier });
      const lang = /\.lua$/i.test(defs[0]?.path || '') ? 'lua' : 'gsc';
      if (defs.length) text = defs.slice(0, 3).map((d) => '```' + lang + '\n' + d.signature + '\n```\n' + `${d.path}:${d.line}`).join('\n\n') + (defs.length > 3 ? `\n\n…and ${defs.length - 3} more` : '') + '\n\nF12 or Ctrl+click to go there';
    } catch { /* not indexed */ }
    hoverCache.current.set(k, text);
    return text;
  };
  // Ctrl+S: follows the Smart Save setting. Ctrl+Shift+S / button: always the dialog.
  const requestSave = () => {
    const { text: t, saved: s0 } = latest.current;
    if (t === null || t === s0 || saving) return;
    const mode = getSaveMode();
    if (mode === 'ask' || (mode === 'ask-large' && changedLines(s0, t) >= getLargeThreshold())) setSmart(true);
    else if (mode === 'auto') save(undefined, { backupBefore: true, backupAfter: false, name: '', description: '', tags: '' });
    else save();
  };
  const save = async (overwriteSha?: string, backup?: SmartSaveChoice) => {
    const { text: t, sha: expected } = latest.current;
    if (t === null || saving) return;
    setSaving(true);
    try {
      const args = { id: fileId, text: t, expectedSha: overwriteSha || expected, eol: doc.data.eol, bom: doc.data.bom };
      const r: any = backup ? await call('versions.smartSave', { ...args, ...backup }) : await call('files.writeText', args);
      setSha(r.sha256);
      setSaved(t);
      const kept = [r.beforeName && `“${r.beforeName}” (before your edits)`, r.afterName && `“${r.afterName}” (what you saved)`].filter(Boolean);
      toast(backup && (backup.backupBefore || backup.backupAfter)
        ? `Saved with backup${kept.length ? `: ${kept.join(' and ')}` : ''} — see the file’s History`
        : 'Saved — previous version kept in History', 'success');
      onSaved();
    } catch (e: any) {
      const m = /^CONFLICT:([0-9a-f]{64}):(.*)$/s.exec(e.message);
      if (m) {
        const overwrite = await confirm({ title: 'File changed on disk', confirm: 'Overwrite with my version', danger: true,
          body: <>Another program changed <span className="mono">{doc.data.path}</span> after you opened it. Overwrite it with your version? The other program's version will be kept in History, so nothing is lost. Choose Cancel to keep editing (you can copy your changes, then close and reopen to see the new version).</> });
        if (overwrite) { setSaving(false); return save(m[1], backup); }
      } else toast(e.message, 'error');
    } finally {
      setSaving(false);
    }
  };

  if (doc.error) return (
    <Empty icon={<LuTriangleAlert />} title="Can't edit this file">{doc.error}<button className="btn" onClick={onClose}><LuArrowLeft /> Back</button></Empty>
  );
  if (text === null) return <div className="doc muted">Opening…</div>;
  return (
    <div className="editor-pane">
      <div className="panehead">
        <button className="btn ghost sm" onClick={onClose} title="Back to file notes"><LuArrowLeft /></button>
        <div className="grow" style={{ minWidth: 110 }}>
          <div className="row" style={{ gap: 6 }}><b>{info.data?.name || doc.data.path.split('/').pop()}</b>{dirty && <span className="dirty-dot" title="Unsaved changes" />}{doc.data.readonly && <span className="badge">read-only</span>}</div>
          <div className="small dim mono" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{doc.data.path} · {doc.data.eol.toUpperCase()}{doc.data.bom ? ' · BOM' : ''}</div>
        </div>
        <span className="small dim" style={{ minWidth: 0, flexShrink: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={dirty ? 'Unsaved — Ctrl+S to save, Ctrl+Shift+S to save with a backup' : undefined}>{saving ? 'Saving…' : dirty ? 'Unsaved — Ctrl+S to save' : 'Saved'}</span>
        <button className="btn sm" disabled={!dirty || saving || doc.data.readonly} onClick={() => setSmart(true)} title="Describe the change and keep a named backup (Ctrl+Shift+S)"><LuShieldCheck /> Save with backup…</button>
        <button className="btn ghost sm icon" title={wide ? 'Show file list' : 'Full width'} onClick={onToggleWide}>{wide ? <LuMinimize2 /> : <LuMaximize2 />}</button>
        <button className="btn primary sm" disabled={!dirty || saving || doc.data.readonly} onClick={requestSave}><LuSave /> Save</button>
      </div>
      <CodeEditor value={text} ext={info.data?.ext || doc.data.path.split('.').pop() || ''} readOnly={doc.data.readonly} onChange={setText} onSave={requestSave} onReady={(h) => { edHandle.current = h; if (pendingLine.current) { h.revealLine(pendingLine.current); pendingLine.current = null; } }}
        onGoToDefinition={isScript ? goToDefinition : undefined} onHover={isScript ? hover : undefined} onSaveWithBackup={() => { if (latest.current.text !== latest.current.saved) setSmart(true); }} />
      {smart && <SmartSaveDialog fileId={fileId} fileName={info.data?.name || doc.data.path.split('/').pop()} projectName={ctx.project.name} projectId={ctx.project.id}
        relDir={doc.data.path.includes('/') ? doc.data.path.slice(0, doc.data.path.lastIndexOf('/')) : ''} linesChanged={changedLines(saved, text)}
        onCancel={() => { setSmart(false); refocus(); }} onSaveNormally={() => { setSmart(false); save().then(refocus); }} onSaveWithBackup={(c) => { setSmart(false); save(undefined, c).then(refocus); }} />}
    </div>
  );
}
