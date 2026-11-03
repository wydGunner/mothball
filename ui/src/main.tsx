import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { LuLayoutDashboard, LuSearch, LuSettings, LuPanelLeftClose, LuPanelLeftOpen, LuStar, LuClock, LuPlus, LuChevronRight, LuChevronDown, LuX } from 'react-icons/lu';
import { call, isTauri, type Project } from './ipc';
import { ToastProvider, ConfirmProvider, ContextMenuProvider, PromptProvider, ProjectIcon, TooltipLayer, useToast } from './components/common';
import { Dashboard, NewProjectDialog } from './views/Dashboard';
import { ProjectView, type Tab } from './views/Project';
import { SearchView } from './views/Search';
import { SettingsView } from './views/Settings';
import { confirmDiscardProject, unsavedNames } from './unsaved';
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
  | { view: 'project'; id: string; tab?: Tab; focus?: Focus; /** open in a new tab instead of the current one (Ctrl/middle-click) */ newTab?: boolean };

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
  const [projectsLoaded, setProjectsLoaded] = useState(false);
  // open project tabs (ids, in tab order), kept across restarts
  const [openTabs, setOpenTabs] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('pv.openTabs') || '[]'); } catch { return []; } });
  useEffect(() => { try { localStorage.setItem('pv.openTabs', JSON.stringify(openTabs)); } catch { /* ignore */ } }, [openTabs]);
  // the tab/focus each open project was last asked to show (stable objects, so hidden tabs don't re-run them)
  const tabRoutes = useRef<Record<string, { tab?: Tab; focus?: Focus }>>({});
  const [, setDirtyTick] = useState(0);
  useEffect(() => { const f = () => setDirtyTick((n) => n + 1); window.addEventListener('pv:dirty', f); return () => window.removeEventListener('pv:dirty', f); }, []);
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

  const refreshProjects = useCallback(() => { call<Project[]>('projects.list').then((l) => { setProjects(l); setProjectsLoaded(true); }).catch(() => {}); }, []);
  // tabs of removed projects go away
  useEffect(() => { if (projectsLoaded) setOpenTabs((t) => { const n = t.filter((id) => projects.some((p) => p.id === id)); return n.length === t.length ? t : n; }); }, [projects, projectsLoaded]);
  useEffect(refreshProjects, [refreshProjects]);
  useEffect(() => { try { localStorage.setItem('pv.sidebar', collapsed ? '1' : '0'); } catch { /* ignore */ } }, [collapsed]);

  const routeRef = useRef(route);
  routeRef.current = route;
  // the project tab you were last in (reused when you open a project from the dashboard)
  const lastTab = useRef<string | null>(null);
  if (route.view === 'project') lastTab.current = route.id;
  else if (lastTab.current && !openTabs.includes(lastTab.current)) lastTab.current = null;
  const go = useCallback((r: Route) => {
    const cur = routeRef.current;
    if (r.view === 'project') {
      tabRoutes.current[r.id] = { tab: r.tab, focus: r.focus };
      setOpenTabs((t) => {
        if (t.includes(r.id)) return t; // already open: just switch to it
        // like a browser: the project replaces the one in the current tab (or the
        // last one you were in, coming from the dashboard), unless a new tab was
        // asked for or that tab has unsaved edits
        const reuse = cur.view === 'project' ? cur.id : lastTab.current;
        if (!r.newTab && reuse && t.includes(reuse) && !unsavedNames(reuse).length) {
          delete tabRoutes.current[reuse];
          tabRoutes.current[r.id] = { tab: r.tab, focus: r.focus };
          return t.map((id) => (id === reuse ? r.id : id));
        }
        let n = [...t, r.id];
        const max = (() => { try { return Number(localStorage.getItem('pv.maxTabs')) || 10; } catch { return 10; } })();
        // too many open: close the oldest tab without unsaved edits (never the one being opened)
        while (n.length > max) {
          const drop = n.find((id) => id !== r.id && !unsavedNames(id).length);
          if (!drop) break;
          n = n.filter((id) => id !== drop);
          delete tabRoutes.current[drop];
        }
        return n;
      });
    }
    setHistory((h) => [...h.slice(-30), cur]);
    setRoute(r);
  }, []);
  const historyRef = useRef<Route[]>([]);
  historyRef.current = history;
  const back = useCallback(() => {
    const h = historyRef.current;
    if (!h.length) return;
    const prev = h[h.length - 1];
    if (prev.view === 'project' && !openTabsRef.current.includes(prev.id)) setOpenTabs((t) => [...t, prev.id]);
    setRoute(prev);
    setHistory(h.slice(0, -1));
  }, []);
  const openTabsRef = useRef(openTabs);
  openTabsRef.current = openTabs;

  /** Closes a project tab (asking about unsaved edits); shows its neighbour if it was in front. */
  const closeTab = useCallback((id: string) => {
    if (!confirmDiscardProject(id)) return;
    const t = openTabsRef.current;
    const i = t.indexOf(id);
    const rest = t.filter((x) => x !== id);
    setOpenTabs(rest);
    delete tabRoutes.current[id];
    setHistory((h) => h.filter((r) => !(r.view === 'project' && r.id === id)));
    const cur = routeRef.current;
    if (cur.view === 'project' && cur.id === id) {
      const next = rest[i] || rest[i - 1];
      setRoute(next ? { view: 'project', id: next } : { view: 'dashboard' });
    }
  }, []);
  const cycleTab = useCallback((dir: 1 | -1) => {
    const t = openTabsRef.current;
    if (!t.length) return;
    const cur = routeRef.current;
    const i = cur.view === 'project' ? t.indexOf(cur.id) : -1;
    const next = i < 0 ? (dir === 1 ? t[0] : t[t.length - 1]) : t[(i + dir + t.length) % t.length];
    setHistory((h) => [...h.slice(-30), cur]);
    setRoute({ view: 'project', id: next });
  }, []);
  const nav: Nav = useMemo(() => ({ go, refreshProjects, projects }), [go, refreshProjects, projects]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette(true); }
      else if (mod && e.shiftKey && e.key.toLowerCase() === 'f') { e.preventDefault(); go({ view: 'search' }); }
      else if (mod && e.key.toLowerCase() === 'n' && !e.shiftKey) { e.preventDefault(); setNewProject(true); }
      else if (mod && e.key.toLowerCase() === 'b') { e.preventDefault(); setCollapsed((c) => !c); }
      else if (e.altKey && e.key === 'ArrowLeft') { e.preventDefault(); back(); }
      else if (e.ctrlKey && e.key === 'Tab') { e.preventDefault(); cycleTab(e.shiftKey ? -1 : 1); }
      else if (mod && !e.shiftKey && e.key.toLowerCase() === 'w') { e.preventDefault(); const r = routeRef.current; if (r.view === 'project') closeTab(r.id); }
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [go, back, cycleTab, closeTab]);

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
  // Groups list all their projects A→Z (collapsible).
  const groupNames = [...new Set(projects.filter((p) => !p.favorite && p.group).map((p) => p.group))].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }));
  // The 8 most recently opened projects, grouped ones included (their group may be collapsed),
  // listed alphabetically so opening one doesn't reshuffle the list. Favorites are always shown above.
  const recent = [...projects].filter((p) => p.lastOpenedAt && !p.favorite).sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0)).slice(0, 8)
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true }));
  const [sideClosed, setSideClosed] = useState<string[]>(() => { try { return JSON.parse(localStorage.getItem('pv.sideCollapsed') || '[]'); } catch { return []; } });
  const toggleSide = (g: string) => setSideClosed((c) => { const n = c.includes(g) ? c.filter((x) => x !== g) : [...c, g]; try { localStorage.setItem('pv.sideCollapsed', JSON.stringify(n)); } catch { /* ignore */ } return n; });

  const projLink = (p: Project) => (
    <button key={p.id} className={cx('navitem', route.view === 'project' && route.id === p.id && 'active')} title={`${p.name}\nCtrl+click or middle-click: open in a new tab`}
      onClick={(e) => go({ view: 'project', id: p.id, newTab: e.ctrlKey || e.metaKey })}
      onAuxClick={(e) => { if (e.button === 1) { e.preventDefault(); go({ view: 'project', id: p.id, newTab: true }); } }} onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}>
      <ProjectIcon category={p.category} icon={p.icon} color={p.color} size={18} /><span className="label">{p.name}</span>
    </button>
  );

  return (
    <div className="app">
      <aside className={cx('sidebar', collapsed && 'collapsed')}>
        <div className="brand" data-tauri-drag-region><svg className="logo" viewBox="0 0 64 64" aria-hidden="true"><g fill="currentColor"><path d="M28.5 30.5 17.5 33.6Q13.1 35.6 14.1 40.4 15.5 45.2 20.7 43.6 25.5 41.8 28.5 37.2Z" fill="var(--accent)" /><path d="M35.5 30.5 46.5 33.6Q50.9 35.6 49.9 40.4 48.5 45.2 43.3 43.6 38.5 41.8 35.5 37.2Z" fill="var(--accent)" /><path d="M28.5 24.2 5.9 19.6Q3.1 19.2 4.1 21.9L11.5 34.6Q12.7 36.6 15.1 36.1L28.5 33.4Z" /><path d="M35.5 24.2 58.1 19.6Q60.9 19.2 59.9 21.9L52.5 34.6Q51.3 36.6 48.9 36.1L35.5 33.4Z" /><path d="M32 18.6C34.8 21.6 35.7 27.8 35.3 35.6 34.9 42 33.5 47.6 32 51.4 30.5 47.6 29.1 42 28.7 35.6 28.3 27.8 29.2 21.6 32 18.6Z" /></g><path d="M31.2 21Q29.6 15.6 24 13.2M32.8 21Q34.4 15.6 40 13.2" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" /></svg><span>Mothball</span></div>
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
            <button className={cx(route.view === 'dashboard' && 'cur')} onClick={() => go({ view: 'dashboard' })}>Projects</button>
            {route.view === 'search' && <><LuChevronRight /><span className="cur">Search</span></>}
            {route.view === 'settings' && <><LuChevronRight /><span className="cur">Settings</span></>}
          </div>
          {openTabs.length > 0 && (
            <div className="ptabs" role="tablist" aria-label="Open projects" data-tauri-drag-region>
              {openTabs.map((id) => {
                const p = projects.find((x) => x.id === id);
                if (!p) return null;
                const on = route.view === 'project' && route.id === id;
                const unsaved = unsavedNames(id).length > 0;
                return (
                  <div key={id} role="tab" aria-selected={on} tabIndex={0} className={cx('ptab', on && 'on')} title={p.name + (unsaved ? ' — unsaved changes' : '')}
                    onClick={() => go({ view: 'project', id })} onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); go({ view: 'project', id }); } }}
                    onAuxClick={(e) => { if (e.button === 1) { e.preventDefault(); closeTab(id); } }} onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}>
                    <ProjectIcon category={p.category} icon={p.icon} color={p.color} size={16} />
                    <span className="ptab-name">{p.name}</span>
                    {unsaved && <span className="ptab-dirty" aria-label="unsaved changes" />}
                    <button className="ptab-x" aria-label={`Close ${p.name}`} title="Close (Ctrl+W)" onClick={(e) => { e.stopPropagation(); closeTab(id); }}><LuX size={12} /></button>
                  </div>
                );
              })}
            </div>
          )}
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
          {/* every open project stays mounted; only the one in front is shown */}
          {openTabs.map((id) => {
            const on = route.view === 'project' && route.id === id;
            const r = tabRoutes.current[id] || (tabRoutes.current[id] = {});
            return <div key={id} className="ptab-pane" style={{ display: on ? 'contents' : 'none' }}><ProjectView nav={nav} id={id} tab={r.tab} focus={r.focus} active={on} /></div>;
          })}
        </div>
      </main>
      {palette && <CommandPalette nav={nav} route={route} onClose={() => setPalette(false)} onNewProject={() => setNewProject(true)} />}
      {newProject && <NewProjectDialog onClose={() => setNewProject(false)} onCreated={(p) => { setNewProject(false); refreshProjects(); go({ view: 'project', id: p.id, tab: 'files' }); }} />}
    </div>
  );
}

// Monaco cancels in-flight work (hover, diff computation) when an editor closes and
// leaves the rejection unhandled. Its CancellationError is expected; ignore only that.
window.addEventListener('unhandledrejection', (e) => {
  const r = e.reason;
  if (r && r.name === 'Canceled' && r.message === 'Canceled') e.preventDefault();
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ToastProvider><ConfirmProvider><PromptProvider><ContextMenuProvider><FileDialogsProvider><App /><TooltipLayer /></FileDialogsProvider></ContextMenuProvider></PromptProvider></ConfirmProvider></ToastProvider>
  </StrictMode>,
);
