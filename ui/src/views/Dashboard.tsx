import { useMemo, useState } from 'react';
import { LuLayoutGrid, LuList, LuPlus, LuFolderOpen, LuFolderPlus, LuStar, LuTriangleAlert, LuHardDrive, LuFiles, LuListTodo, LuDownload, LuLink, LuDatabase } from 'react-icons/lu';
import { call, pickFolder, revealPath, isTauri, type Project } from '../ipc';
import { Empty, Modal, ProjectIcon, StatusPill, useAction, useContextMenu, useLoad, useConfirm } from '../components/common';
import { ago, bytes, CATEGORY_LABELS, cx } from '../format';
import type { Nav } from '../main';

type Sort = 'recent' | 'name' | 'updated' | 'created' | 'size' | 'progress';

export function Dashboard({ nav, onNew }: { nav: Nav; onNew: () => void }) {
  const [mode, setMode] = useState<'grid' | 'list'>(() => (localStorage.getItem('pv.dash') as any) || 'grid');
  const [status, setStatus] = useState('');
  const [category, setCategory] = useState('');
  const [tag, setTag] = useState('');
  const [text, setText] = useState('');
  const [sort, setSort] = useState<Sort>('recent');
  const [favOnly, setFavOnly] = useState(false);
  const [importPV, setImportPV] = useState(false);
  const act = useAction();
  const menu = useContextMenu();
  const confirm = useConfirm();
  const statuses = useLoad(() => call<{ name: string; color: string }[]>('projects.statuses'), [nav.projects.length]);
  const colors = Object.fromEntries((statuses.data || []).map((s) => [s.name, s.color]));
  const allTags = useMemo(() => [...new Set(nav.projects.flatMap((p) => p.tags))].sort(), [nav.projects]);

  const list = useMemo(() => {
    const t = text.toLowerCase();
    const out = nav.projects.filter((p) =>
      (!status || p.status === status) && (!category || p.category === category) && (!tag || p.tags.includes(tag)) && (!favOnly || p.favorite) &&
      (!t || p.name.toLowerCase().includes(t) || p.description.toLowerCase().includes(t)));
    const by: Record<Sort, (a: Project, b: Project) => number> = {
      recent: (a, b) => (b.lastOpenedAt || b.updatedAt) - (a.lastOpenedAt || a.updatedAt),
      name: (a, b) => a.name.localeCompare(b.name),
      updated: (a, b) => (b.latestActivityAt || b.updatedAt) - (a.latestActivityAt || a.updatedAt),
      created: (a, b) => b.createdAt - a.createdAt,
      size: (a, b) => b.bytes - a.bytes,
      progress: (a, b) => b.progress - a.progress,
    };
    return out.sort((a, b) => Number(b.favorite) - Number(a.favorite) || by[sort](a, b));
  }, [nav.projects, status, category, tag, text, sort, favOnly]);

  const setMode2 = (m: 'grid' | 'list') => { setMode(m); localStorage.setItem('pv.dash', m); };
  const toggleFav = async (p: Project) => { await act(() => call('projects.update', { id: p.id, patch: { favorite: !p.favorite } })); nav.refreshProjects(); };
  const ctx = (e: React.MouseEvent, p: Project) => menu(e, [
    { label: 'Open', icon: <LuFolderOpen />, onClick: () => nav.go({ view: 'project', id: p.id }) },
    { label: p.favorite ? 'Remove from favorites' : 'Add to favorites', icon: <LuStar />, onClick: () => toggleFav(p) },
    { label: 'Show folder in Explorer', icon: <LuHardDrive />, onClick: () => act(() => revealPath(p.rootPath)), disabled: !isTauri },
    { label: 'Project notes', onClick: () => nav.go({ view: 'project', id: p.id, tab: 'notes' }) },
    { sep: true, label: '' },
    { label: 'Remove from vault…', danger: true, onClick: async () => {
      const ok = await confirm({ title: `Remove “${p.name}”?`, danger: true, confirm: 'Remove project',
        body: p.mode === 'linked'
          ? <>The project, its notes, file documentation and tasks are removed from Project Vault. <b>Your folder {p.rootPath} is not touched.</b> Make a backup first if you may want this metadata back.</>
          : <>The project is removed from Project Vault. Its managed folder is <b>kept</b> on disk at {p.rootPath}; delete it yourself if you no longer need it.</> });
      if (ok) { await act(() => call('projects.remove', { id: p.id, deleteManagedFiles: false }), 'Project removed'); nav.refreshProjects(); }
    } },
  ]);

  if (!nav.projects.length) {
    return (
      <div className="page">
        <Empty icon={<LuDatabase />} title="Your vault is empty">
          <p style={{ maxWidth: 520 }}>Add a project to start. <b>Link</b> an existing folder (nothing is moved or renamed) or create a <b>managed</b> project whose files Project Vault stores for you.</p>
          <div className="row"><button className="btn primary" onClick={onNew}><LuPlus /> New project</button><button className="btn" onClick={() => setImportPV(true)}><LuDownload /> Import from Print Vault</button></div>
        </Empty>
        {importPV && <ImportPrintVault onClose={() => setImportPV(false)} onDone={nav.refreshProjects} />}
      </div>
    );
  }

  return (
    <div className="page">
      <div className="row"><h1>Projects</h1><span className="muted">{list.length} of {nav.projects.length}</span>
        <div className="right row">
          <button className="btn" onClick={() => setImportPV(true)}><LuDownload /> Import Print Vault</button>
          <button className="btn primary" onClick={onNew}><LuPlus /> New project</button>
        </div>
      </div>
      <div className="toolbar">
        <input className="input" style={{ width: 220 }} placeholder="Filter projects…" value={text} onChange={(e) => setText(e.target.value)} />
        <select className="input" style={{ width: 140 }} value={status} onChange={(e) => setStatus(e.target.value)} aria-label="Status filter">
          <option value="">All statuses</option>{(statuses.data || []).map((s) => <option key={s.name}>{s.name}</option>)}
        </select>
        <select className="input" style={{ width: 150 }} value={category} onChange={(e) => setCategory(e.target.value)} aria-label="Category filter">
          <option value="">All categories</option>{Object.entries(CATEGORY_LABELS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
        </select>
        {allTags.length > 0 && <select className="input" style={{ width: 130 }} value={tag} onChange={(e) => setTag(e.target.value)} aria-label="Tag filter">
          <option value="">All tags</option>{allTags.map((t) => <option key={t}>{t}</option>)}
        </select>}
        <button className={cx('btn', favOnly && 'primary')} onClick={() => setFavOnly(!favOnly)}><LuStar /> Favorites</button>
        <select className="input" style={{ width: 160 }} value={sort} onChange={(e) => setSort(e.target.value as Sort)} aria-label="Sort">
          <option value="recent">Recently opened</option><option value="updated">Latest activity</option><option value="name">Name</option>
          <option value="created">Newest</option><option value="size">Storage used</option><option value="progress">Progress</option>
        </select>
        <div className="seg right"><button className={cx(mode === 'grid' && 'on')} onClick={() => setMode2('grid')} title="Grid"><LuLayoutGrid /></button><button className={cx(mode === 'list' && 'on')} onClick={() => setMode2('list')} title="List"><LuList /></button></div>
      </div>

      {mode === 'grid' ? (
        <div className="pgrid">
          {list.map((p) => (
            <div key={p.id} className="card pcard" onClick={() => nav.go({ view: 'project', id: p.id })} onContextMenu={(e) => ctx(e, p)}>
              <button className={cx('star fav', p.favorite && 'on')} onClick={(e) => { e.stopPropagation(); toggleFav(p); }} title="Favorite"><LuStar fill={p.favorite ? 'currentColor' : 'none'} /></button>
              <div className="head">
                <ProjectIcon category={p.category} icon={p.icon} color={p.color} />
                <div className="grow" style={{ paddingRight: 22 }}>
                  <div className="name">{p.name}</div>
                  <div className="row small" style={{ gap: 6 }}><StatusPill status={p.status} colors={colors} /><span className="dim">{CATEGORY_LABELS[p.category]}</span>{p.mode === 'linked' && <span className="dim" title={p.rootPath}><LuLink size={11} /></span>}</div>
                </div>
              </div>
              <div className="desc">{p.description || <span className="dim">No description</span>}</div>
              {p.tags.length > 0 && <div className="row wrap" style={{ gap: 4 }}>{p.tags.slice(0, 5).map((t) => <span className="tag" key={t}>{t}</span>)}</div>}
              <div className="progress" title={`${p.progress}%`}><div style={{ width: `${p.progress}%` }} /></div>
              <div className="stats">
                <span title="Files"><LuFiles size={12} /> {p.fileCount}</span><span>{bytes(p.bytes)}</span>
                {p.openTasks > 0 && <span title="Open tasks"><LuListTodo size={12} /> {p.openTasks}</span>}
                {!p.rootAvailable && <span style={{ color: 'var(--bad)' }} title="Folder not reachable"><LuTriangleAlert size={12} /> offline</span>}
                {(p.changedFiles > 0 || p.pendingReviews > 0) && <span style={{ color: 'var(--warn)' }}>{p.changedFiles + p.pendingReviews} to review</span>}
              </div>
              <div className="small dim" style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{p.latestActivity ? `${p.latestActivity} · ${ago(p.latestActivityAt)}` : `Created ${ago(p.createdAt)}`}</div>
            </div>
          ))}
        </div>
      ) : (
        <table className="list">
          <thead><tr><th style={{ width: 28 }} /><th>Name</th><th>Status</th><th>Category</th><th>Progress</th><th>Files</th><th>Size</th><th>Latest activity</th><th>Created</th></tr></thead>
          <tbody>
            {list.map((p) => (
              <tr key={p.id} className="click" onClick={() => nav.go({ view: 'project', id: p.id })} onContextMenu={(e) => ctx(e, p)}>
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
      {!list.length && <Empty title="No projects match these filters" />}
      {importPV && <ImportPrintVault onClose={() => setImportPV(false)} onDone={nav.refreshProjects} />}
    </div>
  );
}

export function NewProjectDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (p: Project) => void }) {
  const [mode, setMode] = useState<'linked' | 'managed'>('linked');
  const [name, setName] = useState('');
  const [desc, setDesc] = useState('');
  const [category, setCategory] = useState('software');
  const [status, setStatus] = useState('Active');
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const act = useAction();
  const browse = async () => {
    const p = await pickFolder('Choose the project folder to link');
    if (p) { setPath(p); if (!name) setName(p.split(/[\\/]/).filter(Boolean).pop() || ''); }
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
          ? 'Project Vault indexes the folder in place. Your files are never moved, renamed or reorganized.'
          : 'Project Vault creates a storage folder for this project. Import or drop files into it.'}
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
          <select className="input" value={category} onChange={(e) => setCategory(e.target.value)}>{Object.entries(CATEGORY_LABELS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
          {category === 'game' && <span className="small dim">Unity Library/, Temp/, Logs/, obj/ and Build folders are skipped by default.</span>}
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
  const groups = useLoad(() => call<{ id: string; label: string; exts: string[] }[]>('profiles.groups', { category }), [category]);
  if (!groups.data?.length) return <p className="small dim" style={{ margin: 0 }}>All file types are treated equally for this project type.</p>;
  return (
    <div className="typefocus">
      <div className="small muted" style={{ marginBottom: 6 }}>The Files view will put these first{category === '3d-printing' || category === 'electronics' ? ', and earlier versions of them are kept so you can compare and roll back' : ''}:</div>
      {groups.data.map((g) => <div key={g.id} className="row small" style={{ gap: 6, padding: '2px 0' }}><b style={{ width: 130 }}>{g.label}</b><span className="dim mono">{g.exts.slice(0, 8).map((e) => '.' + e).join(' ')}</span></div>)}
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
        <ul>{result.created.map((c: any) => <li key={c.id}>{c.name} <span className="dim">— {c.files} files</span></li>)}</ul>
        {result.skipped.length > 0 && <><p className="muted">Skipped:</p><ul>{result.skipped.map((s: any, i: number) => <li key={i}>{s.name}: <span className="muted">{s.reason}</span></li>)}</ul></>}
      </>}
    </Modal>
  );
}
