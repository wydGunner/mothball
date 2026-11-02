import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { LuLayoutDashboard, LuSearch, LuSettings, LuPanelLeftClose, LuPanelLeftOpen, LuStar, LuClock, LuPlus, LuChevronRight, LuChevronDown } from 'react-icons/lu';
import { call, isTauri, type Project } from './ipc';
import { ToastProvider, ConfirmProvider, ContextMenuProvider, PromptProvider, ProjectIcon, TooltipLayer, useToast } from './components/common';
import { Dashboard, NewProjectDialog } from './views/Dashboard';
import { ProjectView, type Tab } from './views/Project';
import { SearchView } from './views/Search';
import { SettingsView } from './views/Settings';
import { CommandPalette } from './views/Palette';
import { cx } from './format';
import { checkForUpdate, type AvailableUpdate } from './updater';
import { applyTheme, getTheme } from './theme';
import { FileDialogsProvider } from './fileActions';

applyTheme(getTheme());
import { LuDownload } from 'react-icons/lu';

export type Route =
  | { view: 'dashboard' }
  | { view: 'search'; query?: string }
  | { view: 'settings' }
  | { view: 'project'; id: string; tab?: Tab; focus?: Focus };

/** What to show in a project. `action` + `nonce` let the same file be revealed/opened again. */
/** What a project view should bring into view. type 'folder': id is the folder's relative path ('' = project root). action 'history': open the file's history, highlighting versionId when given. */
export interface Focus { type: 'file' | 'note' | 'task' | 'folder'; id: string; action?: 'reveal' | 'edit' | 'history'; versionId?: string; /** with action 'edit': 1-based line to show */ line?: number; nonce?: number }

export interface Nav { go: (r: Route) => void; refreshProjects: () => void; projects: Project[] }

/** Our own black title bar buttons (the window has no system frame). */
function WindowControls() {
  const [max, setMax] = useState(false);
  useEffect(() => {
    let off: (() => void) | undefined;
    import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const w = getCurrentWindow();
      setMax(await w.isMaximized());
      off = await w.onResized(async () => setMax(await w.isMaximized()));
    }).catch(() => {});
    return () => off?.();
  }, []);
  const win = (fn: 'minimize' | 'toggleMaximize' | 'close') => import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow()[fn]());
  return (
    <div className="winctl">
      <button title="Minimize" aria-label="Minimize" onClick={() => win('minimize')}><svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg></button>
      <button title={max ? 'Restore down' : 'Maximize'} aria-label={max ? 'Restore down' : 'Maximize'} onClick={() => win('toggleMaximize')}>
        {max ? <svg viewBox="0 0 10 10"><path d="M2 0h8v8M0 2h8v8H0z" /></svg> : <svg viewBox="0 0 10 10"><path d="M0 0h10v10H0z" /></svg>}
      </button>
      <button className="close" title="Close" aria-label="Close" onClick={() => win('close')}><svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" /></svg></button>
    </div>
  );
}

function App() {
  const [route, setRoute] = useState<Route>({ view: 'dashboard' });
  const [history, setHistory] = useState<Route[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [collapsed, setCollapsed] = useState(() => { try { return localStorage.getItem('pv.sidebar') === '1'; } catch { return false; } });
  const [palette, setPalette] = useState(false);
  const [newProject, setNewProject] = useState(false);
  const [update, setUpdate] = useState<AvailableUpdate | null>(null);
  const [updating, setUpdating] = useState<number | null | false>(false);
  useEffect(() => {
    let off = false;
    const run = () => checkForUpdate().then((u) => { if (!off) setUpdate(u); }).catch(() => { /* offline or no release yet */ });
    const t = setTimeout(run, 4000);
    const i = setInterval(run, 6 * 60 * 60 * 1000);
    (window as any).__pvCheckUpdate = async () => { const u = await checkForUpdate(); setUpdate(u); return u; };
    return () => { off = true; clearTimeout(t); clearInterval(i); };
  }, []);

  const refreshProjects = useCallback(() => { call<Project[]>('projects.list').then(setProjects).catch(() => {}); }, []);
  useEffect(refreshProjects, [refreshProjects]);
  useEffect(() => { try { localStorage.setItem('pv.sidebar', collapsed ? '1' : '0'); } catch { /* ignore */ } }, [collapsed]);

  const go = useCallback((r: Route) => { setRoute((cur) => { setHistory((h) => [...h.slice(-30), cur]); return r; }); }, []);
  const back = useCallback(() => setHistory((h) => { if (h.length) setRoute(h[h.length - 1]); return h.slice(0, -1); }), []);
  const nav: Nav = useMemo(() => ({ go, refreshProjects, projects }), [go, refreshProjects, projects]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette(true); }
      else if (mod && e.shiftKey && e.key.toLowerCase() === 'f') { e.preventDefault(); go({ view: 'search' }); }
      else if (mod && e.key.toLowerCase() === 'n' && !e.shiftKey) { e.preventDefault(); setNewProject(true); }
      else if (mod && e.key.toLowerCase() === 'b') { e.preventDefault(); setCollapsed((c) => !c); }
      else if (e.altKey && e.key === 'ArrowLeft') { e.preventDefault(); back(); }
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [go, back]);

  const byName = (a: Project, b: Project) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true });
  // Scheduled backups: checked at start-up and every 30 minutes while the app is open.
  const toastFn = useToast();
  const toastRef = useRef(toastFn);
  toastRef.current = toastFn;
  useEffect(() => {
    const toast = (m: string, k: any) => toastRef.current(m, k);
    const tick = () => call<any>('backup.runDue').then((r) => {
      if (!r.ran) return;
      if (r.result.ok) toast(`Scheduled backup saved${r.removed?.length ? ` (${r.removed.length} older scheduled backup(s) removed as configured)` : ''}`, 'success');
      else toast(`Scheduled backup failed: ${r.result.error}`, 'error');
    }).catch(() => {});
    const first = setTimeout(tick, 8000);
    const t = setInterval(tick, 30 * 60 * 1000);
    return () => { clearTimeout(first); clearInterval(t); };
  }, []);

  // Background change watching (desktop shell / dev server) bumps a stamp
  // whenever it records changes; refresh what's on screen when it moves.
  useEffect(() => {
    let last: number | null = null;
    const tick = () => {
      if (document.visibilityState !== 'visible') return;
      call<{ stamp: number }>('watch.stamp').then(({ stamp }) => {
        if (last !== null && stamp !== last) { refreshProjects(); window.dispatchEvent(new CustomEvent('pv:changed')); }
        last = stamp;
      }).catch(() => {});
    };
    tick();
    const t = setInterval(tick, 4000);
    return () => clearInterval(t);
  }, [refreshProjects]);

  const favorites = projects.filter((p) => p.favorite).sort(byName);
  // Groups list all their projects A→Z (collapsible); Recent covers the rest.
  const groupNames = [...new Set(projects.filter((p) => !p.favorite && p.group).map((p) => p.group))].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  // The 8 most recently opened projects, listed alphabetically so opening one doesn't reshuffle the list.
  const recent = [...projects].filter((p) => p.lastOpenedAt && !p.favorite && !p.group).sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0)).slice(0, 8)
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true }));
  const [sideClosed, setSideClosed] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('pv.sideCollapsed') || '[]'); } catch { return []; } });
  const toggleSide = (g: string) => setSideClosed((c) => { const n = c.includes(g) ? c.filter((x) => x !== g) : [...c, g]; try { localStorage.setItem('pv.sideCollapsed', JSON.stringify(n)); } catch { /* ignore */ } return n; });
  const current = route.view === 'project' ? projects.find((p) => p.id === route.id) : undefined;

  const projLink = (p: Project) => (
    <button key={p.id} className={cx('navitem', route.view === 'project' && route.id === p.id && 'active')} onClick={() => go({ view: 'project', id: p.id })} title={p.name}>
      <ProjectIcon category={p.category} icon={p.icon} color={p.color} size={18} /><span className="label">{p.name}</span>
    </button>
  );

  return (
    <div className="app">
      <aside className={cx('sidebar', collapsed && 'collapsed')}>
        <div className="brand" data-tauri-drag-region><svg className="logo" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="15" width="18" height="4" rx="1" fill="currentColor" /><rect x="5" y="9.5" width="14" height="4" rx="1" fill="currentColor" opacity=".7" /><rect x="7" y="4" width="10" height="4" rx="1" fill="currentColor" opacity=".45" /></svg><span>Project Vault</span></div>
        <nav>
          <button className={cx('navitem', route.view === 'dashboard' && 'active')} onClick={() => go({ view: 'dashboard' })} title="Dashboard"><LuLayoutDashboard /><span className="label">Dashboard</span></button>
          <button className="navitem" onClick={() => setNewProject(true)} title="New project (Ctrl+N)"><LuPlus /><span className="label">New project</span></button>
        </nav>
        <div className="scroll">
          {favorites.length > 0 && <><div className="section"><LuStar size={10} /> Favorites</div><nav>{favorites.map(projLink)}</nav></>}
          {groupNames.map((g) => {
            const open = !sideClosed.includes(g);
            return (
              <div key={g}>
                <button className="section sectionbtn" aria-expanded={open} onClick={() => toggleSide(g)} title={g}>{open ? <LuChevronDown size={10} /> : <LuChevronRight size={10} />} <span className="label">{g}</span></button>
                {open && <nav>{projects.filter((p) => !p.favorite && p.group === g).sort(byName).map(projLink)}</nav>}
              </div>
            );
          })}
          {recent.length > 0 && <><div className="section"><LuClock size={10} /> Recent</div><nav>{recent.map(projLink)}</nav></>}
        </div>
        <div className="foot">
          <button className={cx('navitem', route.view === 'settings' && 'active')} onClick={() => go({ view: 'settings' })} title="Settings, backups and restore"><LuSettings /><span className="label">Settings</span></button>
          <button className="navitem" onClick={() => setCollapsed((c) => !c)} title="Toggle sidebar (Ctrl+B)">{collapsed ? <LuPanelLeftOpen /> : <LuPanelLeftClose />}<span className="label">Collapse</span></button>
        </div>
      </aside>
      <main className="main">
        <div className="topbar" data-tauri-drag-region>
          <div className="crumbs" data-tauri-drag-region>
            <button onClick={() => go({ view: 'dashboard' })}>Projects</button>
            {route.view === 'project' && current && <><LuChevronRight /><span className="cur">{current.name}</span></>}
            {route.view === 'search' && <><LuChevronRight /><span className="cur">Search</span></>}
            {route.view === 'settings' && <><LuChevronRight /><span className="cur">Settings</span></>}
          </div>
          {update && (
            <button className="btn primary sm" style={{ marginLeft: 'auto' }} disabled={updating !== false} title={update.notes || undefined}
              onClick={async () => { setUpdating(null); try { await update.install((p) => setUpdating(p)); } catch (e: any) { setUpdating(false); alert('Update failed: ' + (e?.message || e)); } }}>
              <LuDownload /> {updating === false ? `Update to v${update.version}` : updating === null ? 'Downloading…' : updating >= 100 ? 'Installing…' : `Downloading ${updating}%`}
            </button>
          )}
          <div className="searchbox" style={update ? { marginLeft: 8 } : undefined} onClick={() => setPalette(true)} role="button" tabIndex={0}><LuSearch /> Search or run a command… <kbd>Ctrl K</kbd></div>
          {isTauri && <WindowControls />}
        </div>
        <div className="content">
          {route.view === 'dashboard' && <Dashboard nav={nav} onNew={() => setNewProject(true)} />}
          {route.view === 'search' && <SearchView nav={nav} initial={route.query} />}
          {route.view === 'settings' && <SettingsView nav={nav} />}
          {route.view === 'project' && <ProjectView key={route.id} nav={nav} id={route.id} tab={route.tab} focus={route.focus} />}
        </div>
      </main>
      {palette && <CommandPalette nav={nav} route={route} onClose={() => setPalette(false)} onNewProject={() => setNewProject(true)} />}
      {newProject && <NewProjectDialog onClose={() => setNewProject(false)} onCreated={(p) => { setNewProject(false); refreshProjects(); go({ view: 'project', id: p.id, tab: 'files' }); }} />}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ToastProvider><ConfirmProvider><PromptProvider><ContextMenuProvider><FileDialogsProvider><App /><TooltipLayer /></FileDialogsProvider></ContextMenuProvider></PromptProvider></ConfirmProvider></ToastProvider>
  </StrictMode>,
);
