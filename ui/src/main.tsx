import { StrictMode, useCallback, useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { LuLayoutDashboard, LuSearch, LuSettings, LuPanelLeftClose, LuPanelLeftOpen, LuStar, LuClock, LuPlus, LuChevronRight } from 'react-icons/lu';
import { call, type Project } from './ipc';
import { ToastProvider, ConfirmProvider, ContextMenuProvider, ProjectIcon } from './components/common';
import { Dashboard, NewProjectDialog } from './views/Dashboard';
import { ProjectView, type Tab } from './views/Project';
import { SearchView } from './views/Search';
import { SettingsView } from './views/Settings';
import { CommandPalette } from './views/Palette';
import { cx } from './format';

export type Route =
  | { view: 'dashboard' }
  | { view: 'search'; query?: string }
  | { view: 'settings' }
  | { view: 'project'; id: string; tab?: Tab; focus?: { type: 'file' | 'note' | 'task'; id: string } };

export interface Nav { go: (r: Route) => void; refreshProjects: () => void; projects: Project[] }

function App() {
  const [route, setRoute] = useState<Route>({ view: 'dashboard' });
  const [history, setHistory] = useState<Route[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [collapsed, setCollapsed] = useState(() => { try { return localStorage.getItem('pv.sidebar') === '1'; } catch { return false; } });
  const [palette, setPalette] = useState(false);
  const [newProject, setNewProject] = useState(false);

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

  const favorites = projects.filter((p) => p.favorite);
  const recent = [...projects].filter((p) => p.lastOpenedAt && !p.favorite).sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0)).slice(0, 8);
  const current = route.view === 'project' ? projects.find((p) => p.id === route.id) : undefined;

  const projLink = (p: Project) => (
    <button key={p.id} className={cx('navitem', route.view === 'project' && route.id === p.id && 'active')} onClick={() => go({ view: 'project', id: p.id })} title={p.name}>
      <ProjectIcon category={p.category} icon={p.icon} color={p.color} size={18} /><span className="label">{p.name}</span>
    </button>
  );

  return (
    <div className="app">
      <aside className={cx('sidebar', collapsed && 'collapsed')}>
        <div className="brand"><div className="logo">PV</div><span>Project Vault</span></div>
        <nav>
          <button className={cx('navitem', route.view === 'dashboard' && 'active')} onClick={() => go({ view: 'dashboard' })} title="Dashboard"><LuLayoutDashboard /><span className="label">Dashboard</span></button>
          <button className={cx('navitem', route.view === 'search' && 'active')} onClick={() => go({ view: 'search' })} title="Search (Ctrl+Shift+F)"><LuSearch /><span className="label">Search</span></button>
          <button className="navitem" onClick={() => setNewProject(true)} title="New project (Ctrl+N)"><LuPlus /><span className="label">New project</span></button>
        </nav>
        <div className="scroll">
          {favorites.length > 0 && <><div className="section"><LuStar size={10} /> Favorites</div><nav>{favorites.map(projLink)}</nav></>}
          {recent.length > 0 && <><div className="section"><LuClock size={10} /> Recent</div><nav>{recent.map(projLink)}</nav></>}
        </div>
        <div className="foot">
          <button className={cx('navitem', route.view === 'settings' && 'active')} onClick={() => go({ view: 'settings' })} title="Settings & backup"><LuSettings /><span className="label">Settings & backup</span></button>
          <button className="navitem" onClick={() => setCollapsed((c) => !c)} title="Toggle sidebar (Ctrl+B)">{collapsed ? <LuPanelLeftOpen /> : <LuPanelLeftClose />}<span className="label">Collapse</span></button>
        </div>
      </aside>
      <main className="main">
        <div className="topbar">
          <div className="crumbs">
            <button onClick={() => go({ view: 'dashboard' })}>Projects</button>
            {route.view === 'project' && current && <><LuChevronRight /><span className="cur">{current.name}</span></>}
            {route.view === 'search' && <><LuChevronRight /><span className="cur">Search</span></>}
            {route.view === 'settings' && <><LuChevronRight /><span className="cur">Settings & backup</span></>}
          </div>
          <div className="searchbox" onClick={() => setPalette(true)} role="button" tabIndex={0}><LuSearch /> Search or run a command… <kbd>Ctrl K</kbd></div>
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
    <ToastProvider><ConfirmProvider><ContextMenuProvider><App /></ContextMenuProvider></ConfirmProvider></ToastProvider>
  </StrictMode>,
);
