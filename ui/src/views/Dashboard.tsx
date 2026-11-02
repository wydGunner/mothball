import { useEffect, useMemo, useRef, useState } from 'react';
import { LuSearch, LuFilter, LuX, LuLayoutGrid, LuList, LuPlus, LuFolderOpen, LuFolderPlus, LuStar, LuTriangleAlert, LuHardDrive, LuFiles, LuListTodo, LuDownload, LuLink, LuDatabase, LuChevronDown, LuChevronRight, LuFolderInput, LuPencilLine, LuImage, LuFileCode, LuUpload, LuCircleCheck, LuHistory } from 'react-icons/lu';
import { call, pickFiles, pickFolder, revealPath, isTauri, type Project } from '../ipc';
import { Dropdown, Empty, Modal, ProjectIcon, StatusPill, useAction, useContextMenu, useLoad, useConfirm, usePrompt } from '../components/common';
import { ago, bytes, CATEGORY_LABELS, cx, hiddenTypes, typeChoices } from '../format';
import type { Nav } from '../main';
import { Welcome } from './Welcome';

type Sort = 'custom' | 'recent' | 'name' | 'updated' | 'created' | 'size' | 'progress';
const SORTS: [Sort, string][] = [['custom', 'My order (drag cards)'], ['recent', 'Recently opened'], ['updated', 'Latest activity'], ['name', 'Name'], ['created', 'Newest'], ['size', 'Storage used'], ['progress', 'Progress']];

export function Dashboard({ nav, onNew }: { nav: Nav; onNew: () => void }) {
  const [mode, setMode] = useState<'grid' | 'list'>(() => (localStorage.getItem('pv.dash') as any) || 'grid');
  const [status, setStatus] = useState('');
  const [category, setCategory] = useState('');
  const [tag, setTag] = useState('');
  const [text, setText] = useState('');
  // Name by default so cards stay put; the choice is remembered.
  const [sort, setSortRaw] = useState<Sort>(() => (localStorage.getItem('pv.dashSort') as Sort) || 'name');
  const setSort = (s: Sort) => { setSortRaw(s); localStorage.setItem('pv.dashSort', s); };
  const [order, setOrderRaw] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('pv.cardOrder') || '[]'); } catch { return []; } });
  const setOrder = (o: string[]) => { setOrderRaw(o); try { localStorage.setItem('pv.cardOrder', JSON.stringify(o)); } catch { /* ignore */ } };
  const [cardDrop, setCardDrop] = useState<{ id: string; after: boolean } | null>(null);
  const [favOnly, setFavOnly] = useState(false);
  const [groupF, setGroupF] = useState<string | null>(null);
  const [collapsed, setCollapsedRaw] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('pv.dashCollapsed') || '[]'); } catch { return []; } });
  const toggleCollapsed = (g: string) => setCollapsedRaw((c) => { const n = c.includes(g) ? c.filter((x) => x !== g) : [...c, g]; try { localStorage.setItem('pv.dashCollapsed', JSON.stringify(n)); } catch { /* ignore */ } return n; });
  const [dragOver, setDragOver] = useState<string | null>(null);
  const [pictureFor, setPictureFor] = useState<Project | null>(null);
  const prompt = usePrompt();
  const [attention, setAttention] = useState(false);
  const [importPV, setImportPV] = useState(false);
  const act = useAction();
  const menu = useContextMenu();
  const confirm = useConfirm();
  const statuses = useLoad(() => call<{ name: string; color: string }[]>('projects.statuses'), [nav.projects.length]);
  const colors = Object.fromEntries((statuses.data || []).map((s) => [s.name, s.color]));
  const allTags = useMemo(() => [...new Set(nav.projects.flatMap((p) => p.tags))].sort(), [nav.projects]);
  const dashStamp = nav.projects.map((p) => `${p.id}:${p.lastScanAt}:${p.fileCount}`).join('|');
  const dash = useLoad(() => call<DashData>('projects.dashboard', { limit: 15 }), [dashStamp]);
  const [drawer, setDrawer] = useState<null | 'attention' | 'recent'>(null);
  const attCount = dash.data?.attention.length || 0;

  const list = useMemo(() => {
    const t = text.toLowerCase();
    const out = nav.projects.filter((p) =>
      (groupF === null || p.group === groupF) && (!status || p.status === status) && (!category || p.category === category) && (!tag || p.tags.includes(tag)) && (!favOnly || p.favorite) &&
      (!attention || (p.staleDocs || 0) + p.pendingReviews + p.missingFiles > 0 || !p.rootAvailable) &&
      (!t || p.name.toLowerCase().includes(t) || p.description.toLowerCase().includes(t)));
    const by: Record<Sort, (a: Project, b: Project) => number> = {
      recent: (a, b) => (b.lastOpenedAt || b.updatedAt) - (a.lastOpenedAt || a.updatedAt),
      name: (a, b) => a.name.localeCompare(b.name),
      updated: (a, b) => (b.latestActivityAt || b.updatedAt) - (a.latestActivityAt || a.updatedAt),
      created: (a, b) => b.createdAt - a.createdAt,
      size: (a, b) => b.bytes - a.bytes,
      progress: (a, b) => b.progress - a.progress,
      custom: (a, b) => { const ia = order.indexOf(a.id), ib = order.indexOf(b.id); return (ia < 0 ? 1e9 : ia) - (ib < 0 ? 1e9 : ib) || a.name.localeCompare(b.name); },
    };
    // in your own order, favorites aren't pulled to the front
    return out.sort((a, b) => (sort === 'custom' ? 0 : Number(b.favorite) - Number(a.favorite)) || by[sort](a, b));
  }, [nav.projects, status, category, tag, text, sort, favOnly, attention, groupF, order]);
  /** Drop one card onto another: it goes before/after it (switching to "My order"), and joins that card's group. */
  const dropOnCard = (dragId: string, target: Project, after: boolean) => {
    if (dragId === target.id) return;
    const ids = list.map((p) => p.id).filter((id) => id !== dragId);
    const at = ids.indexOf(target.id) + (after ? 1 : 0);
    ids.splice(at, 0, dragId);
    const rest = order.filter((id) => !ids.includes(id));
    setOrder([...ids, ...rest]);
    if (sort !== 'custom') setSort('custom');
    const dragged = nav.projects.find((p) => p.id === dragId);
    if (dragged && dragged.group !== target.group) setGroup([dragId], target.group);
  };
  const allGroups = useMemo(() => [...new Set(nav.projects.map((p) => p.group).filter(Boolean))].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' })), [nav.projects]);
  // sections: each group (A→Z), then ungrouped; a flat list when nothing is grouped
  const sections = useMemo(() => {
    if (!allGroups.length) return [{ key: '', label: '', items: list }];
    const out = allGroups.map((g) => ({ key: g, label: g, items: list.filter((p) => p.group === g) })).filter((x) => x.items.length);
    const rest = list.filter((p) => !p.group);
    if (rest.length) out.push({ key: '', label: 'Ungrouped', items: rest });
    return out;
  }, [list, allGroups]);
  const setGroup = async (ids: string[], group: string) => {
    for (const id of ids) await act(() => call('projects.update', { id, patch: { group } }));
    nav.refreshProjects();
  };
  const askGroup = async (p: Project) => {
    const g = await prompt({ title: `Group for “${p.name}”`, label: 'Group name', value: p.group, confirm: 'Save', suggestions: allGroups,
      hint: 'Pick an existing group or type a new name. Leave empty to ungroup.', validate: (v) => (v.trim().length > 60 ? 'At most 60 characters' : null) });
    if (g !== null) setGroup([p.id], g.trim());
  };
  const renameGroup = async (g: string) => {
    const n = await prompt({ title: `Rename group “${g}”`, label: 'New name', value: g, confirm: 'Rename', validate: (v) => (!v.trim() ? 'Enter a name' : v.trim().length > 60 ? 'At most 60 characters' : null) });
    if (n && n.trim() !== g) setGroup(nav.projects.filter((p) => p.group === g).map((p) => p.id), n.trim());
  };
  const ungroupAll = async (g: string) => {
    if (await confirm({ title: `Remove the group “${g}”?`, confirm: 'Remove group', body: <>The projects in it become ungrouped. Nothing else changes.</> })) setGroup(nav.projects.filter((p) => p.group === g).map((p) => p.id), '');
  };
  const activeFilters = [
    favOnly && { key: 'fav', label: 'Favorites', clear: () => setFavOnly(false) },
    groupF !== null && { key: 'g', label: `Group: ${groupF || 'Ungrouped'}`, clear: () => setGroupF(null) },
    attention && { key: 'att', label: 'Needs attention', clear: () => setAttention(false) },
    status && { key: 's', label: `Status: ${status}`, clear: () => setStatus('') },
    category && { key: 'c', label: CATEGORY_LABELS[category], clear: () => setCategory('') },
    tag && { key: 't', label: `#${tag}`, clear: () => setTag('') },
  ].filter(Boolean) as { key: string; label: string; clear: () => void }[];

  const setMode2 = (m: 'grid' | 'list') => { setMode(m); localStorage.setItem('pv.dash', m); };
  const toggleFav = async (p: Project) => { await act(() => call('projects.update', { id: p.id, patch: { favorite: !p.favorite } })); nav.refreshProjects(); };
  const removeProject = async (p: Project) => {
    let deleteFiles = false;
    const managed = p.mode === 'managed';
    const ok = await confirm({
      title: `Remove “${p.name}”?`, danger: true, confirm: 'Remove project',
      body: managed
        ? <>The project, its notes, file documentation and tasks are removed from Mothball. Its files are stored by the app at <span className="mono">{p.rootPath}</span>.</>
        : <>The project, its notes, file documentation and tasks are removed from Mothball. <b>Your folder {p.rootPath} is not touched.</b> Make a backup first if you may want this metadata back.</>,
      checkbox: managed ? {
        label: 'Also delete the project’s files',
        hint: 'The folder goes to the Recycle Bin. Leave this off to keep the files on disk.',
        confirmChecked: 'Remove and delete files',
        onChange: (on) => { deleteFiles = on; },
      } : undefined,
    });
    if (!ok) return;
    await act(() => call('projects.remove', { id: p.id, deleteManagedFiles: deleteFiles }),
      deleteFiles ? 'Project removed; its folder was moved to the Recycle Bin' : 'Project removed');
    nav.refreshProjects();
  };
  const ctx = (e: React.MouseEvent, p: Project) => menu(e, [
    { label: 'Open', icon: <LuFolderOpen />, onClick: () => nav.go({ view: 'project', id: p.id }) },
    { label: p.favorite ? 'Remove from favorites' : 'Add to favorites', icon: <LuStar />, onClick: () => toggleFav(p) },
    { label: p.group ? 'Change group…' : 'Add to a group…', icon: <LuFolderInput />, onClick: () => askGroup(p) },
    { label: 'Change picture…', icon: <LuImage />, onClick: () => setPictureFor(p) },
    ...(p.group ? [{ label: `Remove from “${p.group}”`, onClick: () => setGroup([p.id], '') }] : []),
    { label: 'Show folder in Explorer', icon: <LuHardDrive />, onClick: () => act(() => revealPath(p.rootPath)), disabled: !isTauri },
    { label: 'Project notes', onClick: () => nav.go({ view: 'project', id: p.id, tab: 'notes' }) },
    { sep: true, label: '' },
    { label: 'Remove from vault…', danger: true, onClick: () => removeProject(p) },
  ]);

  if (!nav.projects.length) {
    return (
      <>
        <Welcome nav={nav} onImportPrintVault={() => setImportPV(true)} />
        {importPV && <ImportPrintVault onClose={() => setImportPV(false)} onDone={nav.refreshProjects} />}
      </>
    );
  }

  return (
    <div className="page">
      <div className="toolbar dash-bar">
        <h1 className="dash-title">Projects{nav.projects.length > 0 && <span className="dash-count">{list.length !== nav.projects.length ? `${list.length} of ${nav.projects.length}` : nav.projects.length}</span>}</h1>
        <div className="searchinput"><LuSearch /><input className="input" placeholder="Search projects…" value={text} onChange={(e) => setText(e.target.value)} aria-label="Search projects" /></div>
        <Dropdown label="Filter by" icon={<LuFilter />} badge={activeFilters.length || undefined} sections={[
          { title: 'Show', items: [
            { key: 'all', label: 'All projects', checked: !favOnly && !attention, onSelect: () => { setFavOnly(false); setAttention(false); } },
            { key: 'fav', label: 'Favorites only', checked: favOnly, onSelect: () => setFavOnly(!favOnly) },
            { key: 'att', label: 'Needs attention', hint: 'Missing files, file matches to confirm, notes that may be out of date, or folder offline', checked: attention, onSelect: () => setAttention(!attention) },
          ] },
          ...(allGroups.length ? [{ title: 'Group', items: [...allGroups, ''].map((g) => ({ key: 'g:' + g, label: g || 'Ungrouped', checked: groupF === g, onSelect: () => setGroupF(groupF === g ? null : g) })) }] : []),
          { title: 'Status', items: (statuses.data || []).map((x) => ({ key: x.name, label: x.name, checked: status === x.name, onSelect: () => setStatus(status === x.name ? '' : x.name) })) },
          { title: 'Project type', items: Object.entries(CATEGORY_LABELS).filter(([k]) => nav.projects.some((p) => p.category === k)).map(([k, v]) => ({ key: k, label: v, checked: category === k, onSelect: () => setCategory(category === k ? '' : k) })) },
          { title: 'Tag', items: allTags.map((t) => ({ key: t, label: t, checked: tag === t, onSelect: () => setTag(tag === t ? '' : t) })) },
        ]} />
        <Dropdown label={<>Sort: {SORTS.find((x) => x[0] === sort)?.[1]}</>} sections={[{ items: SORTS.map(([k, l]) => ({ key: k, label: l, checked: sort === k, onSelect: () => setSort(k) })) }]} width={200} />
        {activeFilters.map((f) => <span key={f.key} className="fchip">{f.label}<button aria-label={`Remove filter ${f.label}`} onClick={f.clear}><LuX size={12} /></button></span>)}
        {activeFilters.length > 1 && <button className="btn ghost sm" onClick={() => activeFilters.forEach((f) => f.clear())}>Clear all</button>}
        <div className="seg right"><button className={cx(mode === 'grid' && 'on')} onClick={() => setMode2('grid')} title="Grid"><LuLayoutGrid /></button><button className={cx(mode === 'list' && 'on')} onClick={() => setMode2('list')} title="List"><LuList /></button></div>
        <button className={cx('btn icon dash-pop', drawer === 'attention' && 'on', attCount > 0 && 'alert')} title={attCount ? `Needs attention (${attCount})` : 'Needs attention: all clear'} aria-label="Needs attention" onClick={() => setDrawer(drawer === 'attention' ? null : 'attention')}>
          <LuTriangleAlert />{attCount > 0 && <span className="dash-badge">{attCount}</span>}
        </button>
        <button className={cx('btn icon dash-pop', drawer === 'recent' && 'on')} title="Recently changed files" aria-label="Recently changed" onClick={() => setDrawer(drawer === 'recent' ? null : 'recent')}><LuHistory /></button>
        <div className="btngroup">
          <button className="btn newbtn" onClick={onNew}><LuPlus /> New project</button>
          <button className="btn newbtn icon" title="More ways to add projects" aria-label="More ways to add projects" onClick={(e) => { const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); menu({ clientX: r.right - 220, clientY: r.bottom + 4, preventDefault() {} } as any, [
            { label: 'Import from Print Vault…', icon: <LuDownload />, onClick: () => setImportPV(true) },
          ]); }}><LuChevronDown /></button>
        </div>
      </div>
      {drawer && <DashDrawer which={drawer} data={dash.data} nav={nav} onClose={() => setDrawer(null)} onShowOnly={() => { setAttention(true); setDrawer(null); }} />}

      {sections.map((sec) => {
        const isCollapsed = !!sec.label && collapsed.includes(sec.key || '\u0000');
        return (
          <section key={sec.key || '\u0000'} className={cx('pgroup', dragOver === (sec.key || '\u0000') && 'drop')}
            onDragOver={(e) => { if (sec.label && e.dataTransfer.types.includes('text/pv-project')) { e.preventDefault(); setDragOver(sec.key || '\u0000'); } }}
            onDragLeave={() => setDragOver(null)}
            onDrop={(e) => { const id = e.dataTransfer.getData('text/pv-project'); setDragOver(null); if (id && sec.label) setGroup([id], sec.key); }}>
            {sec.label && (
              <div className="pgroup-head" onContextMenu={(e) => sec.key && menu(e, [
                { label: 'Rename group…', icon: <LuPencilLine />, onClick: () => renameGroup(sec.key) },
                { label: 'Remove group (ungroup its projects)…', onClick: () => ungroupAll(sec.key) },
              ])}>
                <button className="btn ghost sm" aria-expanded={!isCollapsed} onClick={() => toggleCollapsed(sec.key || '\u0000')}>{isCollapsed ? <LuChevronRight /> : <LuChevronDown />} <b>{sec.label}</b> <span className="dim">{sec.items.length}</span></button>
                {sec.key && <button className="btn ghost sm icon" title="Rename group" onClick={() => renameGroup(sec.key)}><LuPencilLine /></button>}
              </div>
            )}
            {!isCollapsed && (<>
      {mode === 'grid' ? (
        <div className="pgrid">
          {sec.items.map((p) => (
            <ProjectCard key={p.id} p={p} colors={colors} onOpen={() => nav.go({ view: 'project', id: p.id })} onMenu={(e) => ctx(e, p)} onFav={() => toggleFav(p)} onPictureChanged={nav.refreshProjects}
              dropSide={cardDrop?.id === p.id ? (cardDrop.after ? 'after' : 'before') : undefined}
              onCardDragOver={(after) => { if (cardDrop?.id !== p.id || cardDrop.after !== after) setCardDrop({ id: p.id, after }); }}
              onCardDrop={(dragId, after) => { setCardDrop(null); dropOnCard(dragId, p, after); }}
              onCardDragEnd={() => setCardDrop(null)} />
          ))}
        </div>
      ) : (
        <table className="list">
          <thead><tr><th style={{ width: 28 }} /><th>Name</th><th>Status</th><th>Category</th><th>Progress</th><th>Files</th><th>Size</th><th>Latest activity</th><th>Created</th></tr></thead>
          <tbody>
            {sec.items.map((p) => (
              <tr key={p.id} className="click" draggable onDragStart={(e) => { e.dataTransfer.setData('text/pv-project', p.id); e.dataTransfer.effectAllowed = 'move'; }} onClick={() => nav.go({ view: 'project', id: p.id })} onContextMenu={(e) => ctx(e, p)}>
                <td><button className={cx('star', p.favorite && 'on')} onClick={(e) => { e.stopPropagation(); toggleFav(p); }}><LuStar fill={p.favorite ? 'currentColor' : 'none'} /></button></td>
                <td><div className="row"><ProjectIcon category={p.category} icon={p.icon} color={p.color} size={22} /><b>{p.name}</b>{p.tags.slice(0, 3).map((t) => <span className="tag" key={t}>{t}</span>)}</div></td>
                <td><StatusPill status={p.status} colors={colors} /></td><td className="muted">{CATEGORY_LABELS[p.category]}</td>
                <td style={{ width: 110 }}><div className="progress"><div style={{ width: `${p.progress}%` }} /></div></td>
                <td>{p.fileCount}{!p.rootAvailable && <LuTriangleAlert color="var(--bad)" title="Folder not reachable" />}</td><td>{bytes(p.bytes)}</td>
                <td className="muted">{p.latestActivity ? `${p.latestActivity} · ${ago(p.latestActivityAt)}` : '—'}</td><td className="muted">{ago(p.createdAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
            </>)}
          </section>
        );
      })}
      {!list.length && <Empty title="No projects match these filters" />}
      {importPV && <ImportPrintVault onClose={() => setImportPV(false)} onDone={nav.refreshProjects} />}
      {pictureFor && <PictureDialog p={pictureFor} onClose={() => setPictureFor(null)} onSaved={() => { setPictureFor(null); nav.refreshProjects(); }} />}
    </div>
  );
}

// card pictures, fetched once per project (until its files or picture settings change)
interface Thumb { src: string; fit?: 'cover' | 'contain'; x?: number; y?: number; mode?: string; fileId?: string }
export const thumbCache = new Map<string, { key: string; t: Thumb | null }>();
const thumbKey = (p: Project) => `${p.fileCount}:${p.bytes}:${JSON.stringify(p.module?.thumb || p.module?.thumbFileId || '')}`;

export function useThumb(p: Project) {
  const key = thumbKey(p);
  const cached = thumbCache.get(p.id);
  const [t, setT] = useState<Thumb | null | undefined>(cached?.key === key ? cached.t : undefined);
  useEffect(() => {
    const c = thumbCache.get(p.id);
    if (c?.key === key) { setT(c.t); return; }
    let alive = true;
    call<Thumb | null>('projects.thumbnail', { id: p.id })
      .then(async (r) => (r && (r as any).mode !== 'file' && (r as any).mode !== 'custom' && !(await looksUsable(r.src)) ? null : r))
      .then((r) => { thumbCache.set(p.id, { key, t: r }); if (alive) setT(r); }, () => alive && setT(null));
    return () => { alive = false; };
  }, [p.id, key]);
  return t;
}

/** Automatic picks only: skip images that are tiny or almost entirely dark
 *  (a black texture or a menu background makes a card look empty). */
function looksUsable(src: string): Promise<boolean> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      if (img.naturalWidth < 96 || img.naturalHeight < 64) return resolve(false);
      try {
        const c = document.createElement('canvas');
        c.width = 32; c.height = 32;
        const g = c.getContext('2d');
        if (!g) return resolve(true);
        g.drawImage(img, 0, 0, 32, 32);
        const d = g.getImageData(0, 0, 32, 32).data;
        let lum = 0, bright = 0, seen = 0;
        for (let i = 0; i < d.length; i += 4) {
          if (d[i + 3] < 16) continue; // transparent pixels don't count
          const l = 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
          lum += l; seen++; if (l > 60) bright++;
        }
        if (!seen) return resolve(false);
        resolve(lum / seen > 28 || bright / seen > 0.08);
      } catch { resolve(true); }
    };
    img.onerror = () => resolve(false);
    img.src = src;
  });
}

const imgStyle = (t: Thumb): React.CSSProperties => ({ objectFit: t.fit || 'cover', objectPosition: `${t.x ?? 50}% ${t.y ?? 50}%` });

/** Choose and frame the picture on a project's card. */
export function PictureDialog({ p, onClose, onSaved }: { p: Project; onClose: () => void; onSaved: () => void }) {
  const cur = useLoad(() => call<Thumb | null>('projects.thumbnail', { id: p.id }), [p.id]);
  const cands = useLoad(() => call<{ fileId: string; path: string; ext: string }[]>('projects.thumbnailCandidates', { id: p.id }), [p.id]);
  const [mode, setMode] = useState<string | null>(null);
  const [fileId, setFileId] = useState<string | undefined>(undefined);
  const [src, setSrc] = useState<string | null | undefined>(undefined);
  const [fit, setFit] = useState<'cover' | 'contain'>('cover');
  const [pos, setPos] = useState({ x: 50, y: 50 });
  const [pics, setPics] = useState<Record<string, string | null>>({});
  const act = useAction();
  const drag = useRef<{ x: number; y: number; px: number; py: number } | null>(null);
  useEffect(() => {
    if (cur.data === undefined || mode !== null) return;
    const t = cur.data;
    setMode(t?.mode || (p.module?.thumb?.mode === 'none' ? 'none' : 'auto'));
    setFileId(t?.fileId); setSrc(t?.src ?? null); setFit(t?.fit || 'cover'); setPos({ x: t?.x ?? 50, y: t?.y ?? 50 });
  }, [cur.data, mode, p.module]);
  // load candidate previews a few at a time
  useEffect(() => {
    let alive = true;
    (async () => {
      for (const c of cands.data || []) {
        if (!alive) return;
        if (pics[c.fileId] !== undefined) continue;
        const v = await call<string | null>('files.picture', { id: c.fileId }).catch(() => null);
        if (alive) setPics((m) => ({ ...m, [c.fileId]: v }));
      }
    })();
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cands.data]);
  const fromComputer = async () => {
    const f = (await pickFiles('Choose a picture for this project', [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif'] }]))[0];
    if (!f) return;
    const r = await act(() => call<Thumb>('projects.setThumbnail', { id: p.id, thumb: { path: f, fit, x: pos.x, y: pos.y } }));
    if (r) { setMode('custom'); setFileId(undefined); setSrc(r.src); }
  };
  const save = async () => {
    const thumb: any = { mode, fit, x: Math.round(pos.x), y: Math.round(pos.y) };
    if (mode === 'file') thumb.fileId = fileId;
    // the result is null when the card has no picture, so check for failure explicitly
    const r = await act(() => call('projects.setThumbnail', { id: p.id, thumb }).then(() => true), 'Picture saved');
    if (r) { thumbCache.delete(p.id); onSaved(); }
  };
  const shown = mode === 'none' ? null : src;
  const usable = (cands.data || []).filter((c) => pics[c.fileId] !== null);
  return (
    <Modal wide title={`Picture for ${p.name}`} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={mode === null || (mode === 'file' && !fileId)} onClick={save}>Save</button></>}>
      <div className="picdlg">
        <div>
          <div className={cx('pic-preview', fit === 'cover' && shown && 'draggable')}
            onPointerDown={(e) => { if (fit !== 'cover' || !shown) return; drag.current = { x: e.clientX, y: e.clientY, px: pos.x, py: pos.y }; (e.target as HTMLElement).setPointerCapture(e.pointerId); }}
            onPointerMove={(e) => { const d = drag.current; if (!d) return; const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); setPos({ x: Math.max(0, Math.min(100, d.px - ((e.clientX - d.x) / r.width) * 100)), y: Math.max(0, Math.min(100, d.py - ((e.clientY - d.y) / r.height) * 100)) }); }}
            onPointerUp={() => { drag.current = null; }}>
            {shown ? <img src={shown} alt="" style={imgStyle({ src: shown, fit, x: pos.x, y: pos.y })} draggable={false} /> : <div className="small muted" style={{ padding: 20 }}>{mode === 'none' ? 'No picture: the card shows the most recently changed files.' : cur.data === undefined ? 'Loading…' : 'No picture found in this project.'}</div>}
          </div>
          <div className="small dim" style={{ margin: '6px 0' }}>This is how it appears on the dashboard card.{fit === 'cover' && shown ? ' Drag the picture to choose which part shows.' : ''}</div>
          {shown && <div className="row" style={{ flexWrap: 'wrap' }}>
            <div className="seg"><button className={cx(fit === 'cover' && 'on')} onClick={() => setFit('cover')}>Fill the card</button><button className={cx(fit === 'contain' && 'on')} onClick={() => setFit('contain')}>Show the whole picture</button></div>
            {fit === 'cover' && <button className="btn sm ghost" onClick={() => setPos({ x: 50, y: 50 })}>Center</button>}
          </div>}
        </div>
        <div className="pic-side">
          <div className="pic-options">
            <label className={cx('opcard', mode === 'auto' && 'on')}><input type="radio" checked={mode === 'auto'} onChange={() => { setMode('auto'); setFileId(undefined); call<Thumb | null>('projects.thumbnail', { id: p.id }).then(() => {}); }} />
              <div><b>Automatic</b><div className="small dim">The newest .3mf plate image, else an image named like “cover” or “screenshot”.</div></div></label>
            <label className={cx('opcard', mode === 'none' && 'on')}><input type="radio" checked={mode === 'none'} onChange={() => setMode('none')} />
              <div><b>No picture</b><div className="small dim">Show the most recently changed files instead.</div></div></label>
          </div>
          <div className="row" style={{ margin: '12px 0 6px' }}><b className="grow small">From this project</b><button className="btn sm" onClick={fromComputer}><LuUpload /> Use an image from your computer…</button></div>
          {cands.data && !cands.data.length && <div className="small muted">No images or .3mf files in this project.</div>}
          <div className="pic-grid">
            {usable.map((c) => (
              <button key={c.fileId} className={cx('pic-cand', mode === 'file' && fileId === c.fileId && 'on')} title={c.path}
                onClick={() => { setMode('file'); setFileId(c.fileId); setSrc(pics[c.fileId] || null); }}>
                {pics[c.fileId] ? <img src={pics[c.fileId]!} alt="" /> : <span className="small dim">…</span>}
                <span className="small">{c.path.split('/').pop()}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </Modal>
  );
}

interface DashData {
  attention: { projectId: string; name: string; category: string; icon: string; color: string; reasons: { kind: string; text: string; count?: number }[] }[];
  recent: { fileId: string; path: string; name: string; ext: string; present: boolean; change: string; at: number; projectId: string; projectName: string }[];
}
const CHANGE_WORD: Record<string, string> = { added: 'added', modified: 'changed', moved: 'moved', restored: 'back', 'restored-version': 'restored', renamed: 'renamed', imported: 'imported' };

/** Slide-out panel on the right for Needs attention / Recently changed. */
function DashDrawer({ which, data, nav, onClose, onShowOnly }: { which: 'attention' | 'recent'; data?: DashData; nav: Nav; onClose: () => void; onShowOnly: () => void }) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [onClose]);
  return (
    <>
      <div className="drawer-scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-label={which === 'attention' ? 'Needs attention' : 'Recently changed'}>
        <div className="drawer-h">
          {which === 'attention' ? <LuTriangleAlert /> : <LuHistory />}
          <h2>{which === 'attention' ? 'Needs attention' : 'Recently changed'}</h2>
          {which === 'attention' && (data?.attention.length || 0) > 1 && <button className="btn ghost sm" onClick={onShowOnly}>Show only these projects</button>}
          <button className="btn ghost sm icon" onClick={onClose} title="Close (Esc)" aria-label="Close"><LuX /></button>
        </div>
        <div className="drawer-b" onClick={(e) => { if ((e.target as HTMLElement).closest('.ds-row')) setTimeout(onClose, 0); }}>
          {which === 'attention' ? <AttentionList data={data} nav={nav} /> : <RecentList data={data} nav={nav} />}
        </div>
      </aside>
    </>
  );
}

/** Projects with missing files, offline folders, matches to confirm or stale notes. */
function AttentionList({ data, nav }: { data?: DashData; nav: Nav }) {
  if (!data) return null;
  const fix = (a: DashData['attention'][number]) => {
    const k = a.reasons[0]?.kind;
    nav.go({ view: 'project', id: a.projectId, tab: k === 'staleDocs' || k === 'missing' ? 'files' : 'overview' });
  };
  return data.attention.length ? <>{data.attention.map((a) => (
    <button key={a.projectId} className="ds-row" onClick={() => fix(a)}>
      <ProjectIcon category={a.category} icon={a.icon} color={a.color} size={22} />
      <span className="ds-name">{a.name}</span>
      <span className="ds-why">{a.reasons.map((r) => r.text).join(' · ')}</span>
    </button>
  ))}</> : <div className="ds-empty"><LuCircleCheck /> All clear — no missing files, offline folders or out-of-date notes.</div>;
}

/** Latest file changes across all projects. */
function RecentList({ data, nav }: { data?: DashData; nav: Nav }) {
  if (!data) return null;
  const openFile = (r: DashData['recent'][number]) => nav.go({ view: 'project', id: r.projectId, tab: 'files', focus: r.present ? { type: 'file', id: r.fileId, action: 'reveal', nonce: Date.now() } : undefined });
  return data.recent.length ? <>{data.recent.map((r) => (
    <button key={r.fileId} className="ds-row" onClick={() => openFile(r)} title={`${r.projectName}: ${r.path}`}>
      <LuFileCode className="ds-fi" />
      <span className="ds-name mono">{r.name}</span>
      <span className="ds-why">{r.projectName} · {CHANGE_WORD[r.change] || r.change}</span>
      <span className="ds-when">{ago(r.at)}</span>
    </button>
  ))}</> : <div className="ds-empty">Edits you make to project files — here or in other programs — show up here.</div>;
}

/** A project card: its picture (print plate, cover image) or, for code, the file you touched last. */
function ProjectCard({ p, colors, onOpen, onMenu, onFav, onPictureChanged, dropSide, onCardDragOver, onCardDrop, onCardDragEnd }: {
  p: Project; colors: Record<string, string>; onOpen: () => void; onMenu: (e: React.MouseEvent) => void; onFav: () => void; onPictureChanged: () => void;
  dropSide?: 'before' | 'after'; onCardDragOver: (after: boolean) => void; onCardDrop: (dragId: string, after: boolean) => void; onCardDragEnd: () => void;
}) {
  const thumb = useThumb(p);
  const [picking, setPicking] = useState(false);
  const attention = p.missingFiles + p.pendingReviews + (p.staleDocs || 0);
  return (
    <div className={cx('pcard', p.favorite && 'isfav', dropSide && 'drop-' + dropSide)} style={p.color ? ({ '--pc': p.color } as React.CSSProperties) : undefined}
      tabIndex={0} role="link" aria-label={`Open ${p.name}`}
      draggable onDragStart={(e) => { e.dataTransfer.setData('text/pv-project', p.id); e.dataTransfer.effectAllowed = 'move'; }}
      onDragEnd={onCardDragEnd}
      onDragOver={(e) => { if (!e.dataTransfer.types.includes('text/pv-project')) return; e.preventDefault(); e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); onCardDragOver(e.clientX > r.left + r.width / 2); }}
      onDrop={(e) => { const id = e.dataTransfer.getData('text/pv-project'); if (!id) return; e.preventDefault(); e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); onCardDrop(id, e.clientX > r.left + r.width / 2); }}
      onClick={onOpen} onKeyDown={(e) => { if (e.key === 'Enter') onOpen(); }} onContextMenu={onMenu}>
      <div className={cx('pc-face', thumb ? 'pic' : 'nopic')}>
        <button className="pc-edit btn sm" title="Change the picture" onClick={(e) => { e.stopPropagation(); setPicking(true); }}><LuImage /> Picture</button>
        {thumb ? <>
            {thumb.fit === 'contain' && <img className="pc-backdrop" src={thumb.src} alt="" aria-hidden />}
            <img className="pc-img" src={thumb.src} alt="" loading="lazy" style={imgStyle(thumb)} />
          </>
          : <>
            <div className="pc-watermark" aria-hidden><ProjectIcon category={p.category} icon={p.icon} color={p.color} size={92} /></div>
            {p.recentFiles?.length ? <div className="pc-recent" title="Most recently changed files">
              <div className="pc-recent-h">Recently changed</div>
              {p.recentFiles.map((r) => {
                const d = r.path.includes('/') ? r.path.slice(0, r.path.lastIndexOf('/') + 1) : '';
                return <div key={r.path} className="pc-rf"><LuFileCode /><span className="pc-file">{r.path.slice(d.length)}</span><span className="pc-dir">{d}</span><span className="pc-when">{ago(r.at)}</span></div>;
              })}
            </div> : <div className="pc-recent"><div className="pc-recent-h">No files yet</div></div>}
          </>}
      </div>
      <div className="pc-body">
        <div className="pc-title">
          <span className="name">{p.name}</span>
          <button className={cx('star', p.favorite && 'on')} onClick={(e) => { e.stopPropagation(); onFav(); }} title={p.favorite ? 'Remove from favorites' : 'Add to favorites'} aria-label="Favorite"><LuStar fill={p.favorite ? 'currentColor' : 'none'} /></button>
        </div>
        {p.description && <div className="pc-desc">{p.description}</div>}
        <div className="pc-meta">
          <span>{CATEGORY_LABELS[p.category]}</span>
          <span>{p.fileCount} {p.fileCount === 1 ? 'file' : 'files'}</span>
          <span>{bytes(p.bytes)}</span>
          {p.status !== 'Active' && <StatusPill status={p.status} colors={colors} />}
          {p.openTasks > 0 && <span title="Open tasks"><LuListTodo size={12} /> {p.openTasks}</span>}
          {!p.rootAvailable && <span className="pc-warn" title={`Folder not reachable: ${p.rootPath}`}><LuTriangleAlert size={12} /> offline</span>}
          {attention > 0 && <span className="pc-warn" title={[p.missingFiles && `${p.missingFiles} missing`, p.pendingReviews && `${p.pendingReviews} file matches to confirm`, p.staleDocs && `${p.staleDocs} file notes may be out of date`].filter(Boolean).join(' · ')}><LuTriangleAlert size={12} /> Needs attention</span>}
        </div>
        {p.progress > 0 && <div className="pc-progress" title={`${p.progress}% done`}><div style={{ width: `${p.progress}%` }} /></div>}
      </div>
      {picking && <div onClick={(e) => e.stopPropagation()} onContextMenu={(e) => e.stopPropagation()}><PictureDialog p={p} onClose={() => setPicking(false)} onSaved={() => { setPicking(false); onPictureChanged(); }} /></div>}
    </div>
  );
}

export function NewProjectDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (p: Project) => void }) {
  const [mode, setMode] = useState<'linked' | 'managed'>('linked');
  const [name, setName] = useState('');
  const [desc, setDesc] = useState('');
  const [category, setCategoryRaw] = useState('general');
  const [typeTouched, setTypeTouched] = useState(false);
  const setCategory = (c: string) => setCategoryRaw(c);
  const [status, setStatus] = useState('Active');
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const act = useAction();
  const browse = async () => {
    const p = await pickFolder('Choose the project folder to link');
    if (p) {
      setPath(p);
      if (!name) setName(p.split(/[\\/]/).filter(Boolean).pop() || '');
      const g = await call<{ category: string } | null>('profiles.guess', { path: p }).catch(() => null);
      if (g && !typeTouched && !hiddenTypes().includes(g.category)) setCategory(g.category);
    }
  };
  const create = async () => {
    setBusy(true);
    const p = await act(() => call<Project>('projects.create', { name, description: desc, category, status, linkPath: mode === 'linked' ? path : null }));
    if (p) {
      await act(() => call('files.scan', { projectId: p.id }));
      onCreated(p);
    }
    setBusy(false);
  };
  return (
    <Modal title="New project" onClose={onClose} footer={<>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={busy || !name.trim() || (mode === 'linked' && !path.trim())} onClick={create}>{busy ? 'Creating & indexing…' : 'Create project'}</button>
    </>}>
      <div className="seg" style={{ marginBottom: 14 }}>
        <button className={cx(mode === 'linked' && 'on')} onClick={() => setMode('linked')}><LuLink /> Link existing folder</button>
        <button className={cx(mode === 'managed' && 'on')} onClick={() => setMode('managed')}><LuFolderPlus /> Managed project</button>
      </div>
      <p className="muted small" style={{ marginTop: 0 }}>
        {mode === 'linked'
          ? 'Mothball indexes the folder in place. Your files are never moved, renamed or reorganized.'
          : 'Mothball creates a storage folder for this project. Import or drop files into it.'}
      </p>
      {mode === 'linked' && (
        <div className="field"><label>Folder</label>
          <div className="row"><input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} placeholder="D:\Dev\OMEN" /><button className="btn" onClick={browse}><LuFolderOpen /> Browse…</button></div>
        </div>
      )}
      <div className="field"><label>Name</label><input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="OMEN v2" /></div>
      <div className="field"><label>Description</label><textarea className="input" value={desc} onChange={(e) => setDesc(e.target.value)} rows={2} /></div>
      <div className="grid2">
        <div className="field"><label>Project type</label>
          <select className="input" value={category} onChange={(e) => { setTypeTouched(true); setCategory(e.target.value); }}>{typeChoices(category).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
        </div>
        <div className="field"><label>Status</label>
          <select className="input" value={status} onChange={(e) => setStatus(e.target.value)}>{['Planned', 'Active', 'Paused', 'Completed', 'Archived'].map((s) => <option key={s}>{s}</option>)}</select>
        </div>
      </div>
      <TypeFocus category={category} />
    </Modal>
  );
}

function TypeFocus({ category }: { category: string }) {
  const info = useLoad(() => call<{ groups: { id: string; label: string; exts: string[] }[]; ignores: string; keyExts: string[] }>('profiles.info', { category }), [category]);
  const d = info.data;
  if (!d) return null;
  const ignores = d.ignores.split('\n').filter(Boolean);
  if (!d.groups.length) return <p className="small dim" style={{ margin: 0 }}>All file types are treated equally for this project type.</p>;
  return (
    <div className="typefocus">
      <div className="small muted" style={{ marginBottom: 6 }}>The Files view sorts these into their own groups:</div>
      {d.groups.map((g) => <div key={g.id} className="row small" style={{ gap: 6, padding: '2px 0' }}><b style={{ width: 150, flex: 'none' }}>{g.label}</b><span className="dim mono">{g.exts.slice(0, 8).map((e) => '.' + e).join(' ')}</span></div>)}
      {d.keyExts.length > 0 && <div className="small muted" style={{ marginTop: 8 }}>Earlier versions of project files ({d.keyExts.slice(0, 6).map((e) => '.' + e).join(', ')}{d.keyExts.length > 6 ? '…' : ''}) are kept so you can compare and roll back.</div>}
      {ignores.length > 0 && <div className="small muted" style={{ marginTop: 6 }}>Skipped by default (caches and autosaves): <span className="mono dim">{ignores.slice(0, 6).join('  ')}{ignores.length > 6 ? ' …' : ''}</span>. You can change this per project.</div>}
    </div>
  );
}

function ImportPrintVault({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [path, setPath] = useState('');
  const [result, setResult] = useState<any>(null);
  const [busy, setBusy] = useState(false);
  const act = useAction();
  return (
    <Modal title="Import from Print Vault" onClose={onClose} footer={result ? <button className="btn primary" onClick={onClose}>Done</button> : <>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!path || busy} onClick={async () => { setBusy(true); const r = await act(() => call('legacy.importPrintVault', { path })); setBusy(false); if (r) { setResult(r); onDone(); } }}>{busy ? 'Importing…' : 'Import'}</button>
    </>}>
      {!result ? <>
        <p className="muted" style={{ marginTop: 0 }}>Choose your Print Vault folder (the one containing <code>print_vault.sqlite</code>, e.g. <code>D:\PrintVault\3D Prints Vault</code>). Each Print Vault project becomes a linked 3D-printing project pointing at its existing folder. Material, printer, nozzle, print time and filament are kept; notes become a project note. Print Vault's database is only read.</p>
        <div className="row"><input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} placeholder="D:\PrintVault\3D Prints Vault" /><button className="btn" onClick={async () => { const p = await pickFolder('Print Vault folder'); if (p) setPath(p); }}><LuFolderOpen /> Browse…</button></div>
      </> : <>
        <p><b>{result.created.length}</b> project(s) imported.</p>
        <ul>{result.created.map((c: any) => <li key={c.id}>{c.name} <span className="dim">— {c.files} {c.files === 1 ? 'file' : 'files'}</span></li>)}</ul>
        {result.skipped.length > 0 && <><p className="muted">Skipped:</p><ul>{result.skipped.map((s: any, i: number) => <li key={i}>{s.name}: <span className="muted">{s.reason}</span></li>)}</ul></>}
      </>}
    </Modal>
  );
}
