import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { LuLayoutDashboard, LuFiles, LuNotebookPen, LuListTodo, LuHistory, LuGitBranch, LuRefreshCw, LuTriangleAlert, LuFolderOpen, LuLink, LuStar, LuCircleCheck } from 'react-icons/lu';
import { call, isTauri, pickFolder, revealPath, onFileDrop, type Project, type ScanReport } from '../ipc';
import { Empty, ProjectIcon, StatusPill, useAction, useToast, useConfirm } from '../components/common';
import { ago, cx } from '../format';
import type { Nav } from '../main';
import { Overview } from './Overview';
import { FilesTab } from './Files';
import { NotesTab } from './Notes';
import { TasksTab } from './Tasks';
import { TimelineTab } from './Timeline';
import { GitTab } from './Git';

export type Tab = 'overview' | 'files' | 'notes' | 'tasks' | 'timeline' | 'git';
const TABS: { id: Tab; label: string; icon: ReactElement }[] = [
  { id: 'overview', label: 'Overview', icon: <LuLayoutDashboard /> },
  { id: 'files', label: 'Files', icon: <LuFiles /> },
  { id: 'notes', label: 'Notes', icon: <LuNotebookPen /> },
  { id: 'tasks', label: 'Tasks & bugs', icon: <LuListTodo /> },
  { id: 'timeline', label: 'Timeline', icon: <LuHistory /> },
  { id: 'git', label: 'Git', icon: <LuGitBranch /> },
];

const AUTO_SCAN_MS = 60_000;

export interface ProjectCtx {
  project: Project;
  reload: () => Promise<void>;
  scan: (quiet?: boolean) => Promise<ScanReport | undefined>;
  scanTick: number;
  nav: Nav;
  openFile: (id: string) => void;
  openNote: (id: string) => void;
  openTask: (id: string) => void;
  openFiles: (group?: string) => void;
  openTimeline: () => void;
  fileGroup?: string;
  focus?: { type: 'file' | 'note' | 'task'; id: string };
}

export function ProjectView({ nav, id, tab: initialTab, focus: initialFocus }: { nav: Nav; id: string; tab?: Tab; focus?: { type: 'file' | 'note' | 'task'; id: string } }) {
  const [project, setProject] = useState<Project | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>(initialTab || 'overview');
  const [focus, setFocus] = useState(initialFocus);
  const [scanning, setScanning] = useState(false);
  const [scanTick, setScanTick] = useState(0);
  const [dropping, setDropping] = useState(false);
  const [fileGroup, setFileGroup] = useState<string | undefined>(undefined);
  const toast = useToast();
  const act = useAction();
  const confirm = useConfirm();
  const scanningRef = useRef(false);

  useEffect(() => { if (initialTab) setTab(initialTab); setFocus(initialFocus); }, [initialTab, initialFocus]);

  const reload = useCallback(async () => {
    try { setProject(await call<Project>('projects.get', { id })); setError(null); } catch (e: any) { setError(e.message); }
  }, [id]);

  const scan = useCallback(async (quiet = false) => {
    if (scanningRef.current) return;
    scanningRef.current = true;
    setScanning(true);
    try {
      const r = await call<ScanReport>('files.scan', { projectId: id });
      const changes = r.added + r.modified + r.moved + r.missing + r.restored;
      if (!quiet || changes > 0 || r.needsReview > 0) {
        const parts = [r.added && `${r.added} added`, r.modified && `${r.modified} modified`, r.moved && `${r.moved} moved/renamed`, r.missing && `${r.missing} missing`, r.restored && `${r.restored} restored`, r.needsReview && `${r.needsReview} need review`].filter(Boolean);
        toast(changes || r.needsReview ? `Changes detected: ${parts.join(', ')}` : `Up to date (${r.total} files)`, changes ? 'info' : 'success');
      }
      if (r.errorCount > 0) toast(`${r.errorCount} item(s) could not be read:\n${r.errors.slice(0, 3).join('\n')}`, 'error');
      setScanTick((t) => t + 1);
      await reload();
      nav.refreshProjects();
      return r;
    } catch (e: any) {
      if (!quiet) toast(e.message, 'error');
      await reload();
    } finally {
      scanningRef.current = false;
      setScanning(false);
    }
  }, [id, reload, toast, nav]);

  useEffect(() => { reload().then(() => scan(true)); nav.refreshProjects(); /* eslint-disable-next-line */ }, [id]);
  // Detect external modifications: rescan on focus and periodically. Scans are
  // read-only and skip unchanged files, so they don't disturb other tools.
  useEffect(() => {
    const onFocus = () => scan(true);
    window.addEventListener('focus', onFocus);
    const t = setInterval(() => { if (document.visibilityState === 'visible') scan(true); }, AUTO_SCAN_MS);
    return () => { window.removeEventListener('focus', onFocus); clearInterval(t); };
  }, [scan]);

  // OS drag-and-drop import (desktop app)
  useEffect(() => {
    let off: (() => void) | undefined;
    onFileDrop(async (paths) => {
      if (!paths.length) return;
      const ok = await confirm({ title: `Import ${paths.length} item(s)?`, confirm: 'Copy into project',
        body: <>The files are <b>copied</b> into {project?.mode === 'linked' ? 'the linked folder' : 'managed storage'} (root of the project). Existing files are never overwritten; files whose content already exists in the project are skipped.</> });
      if (!ok) return;
      const r: any = await act(() => call('files.import', { projectId: id, paths, destDir: '' }));
      if (r) { toast(`Imported ${r.imported.length}, skipped ${r.skipped.length}${r.errors.length ? `, ${r.errors.length} errors` : ''}`, r.errors.length ? 'error' : 'success'); setScanTick((t) => t + 1); reload(); }
    }, setDropping).then((f) => { off = f; });
    return () => off?.();
  }, [id, project?.mode, act, confirm, toast, reload]);

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.altKey && /^[1-6]$/.test(e.key)) { e.preventDefault(); setTab(TABS[Number(e.key) - 1].id); }
      if (e.key === 'F5') { e.preventDefault(); scan(false); }
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [scan]);

  if (error) return <div className="page"><Empty icon={<LuTriangleAlert />} title="Project unavailable">{error}<button className="btn" onClick={() => nav.go({ view: 'dashboard' })}>Back to projects</button></Empty></div>;
  if (!project) return <div className="page muted">Loading…</div>;

  const ctx: ProjectCtx = {
    project, reload, scan, scanTick, nav, focus,
    openFile: (fid) => { setFocus({ type: 'file', id: fid }); setTab('files'); },
    openNote: (nid) => { setFocus({ type: 'note', id: nid }); setTab('notes'); },
    openTask: (tid) => { setFocus({ type: 'task', id: tid }); setTab('tasks'); },
    openFiles: (g) => { setFocus(undefined); setFileGroup(g || 'all'); setTab('files'); },
    openTimeline: () => setTab('timeline'),
    fileGroup,
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="row" style={{ padding: '12px 16px 8px', background: 'var(--bg1)', gap: 12 }}>
        <ProjectIcon category={project.category} icon={project.icon} color={project.color} size={34} />
        <div className="grow">
          <div className="row"><h1 style={{ fontSize: 17 }}>{project.name}</h1><StatusPill status={project.status} />
            <button className={cx('star', project.favorite && 'on')} title="Favorite" onClick={async () => { await act(() => call('projects.update', { id, patch: { favorite: !project.favorite } })); reload(); nav.refreshProjects(); }}><LuStar fill={project.favorite ? 'currentColor' : 'none'} /></button>
          </div>
          <div className="small dim mono" style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }} title={project.rootPath}>
            {project.mode === 'linked' ? <LuLink size={11} /> : null} {project.rootPath}
          </div>
        </div>
        <span className="small dim">{project.lastScanAt ? `Checked ${ago(project.lastScanAt)}` : ''}</span>
        <button className="btn" onClick={() => scan(false)} disabled={scanning} title="Re-scan for external changes (F5)"><LuRefreshCw className={scanning ? 'spin' : ''} /> {scanning ? 'Scanning…' : 'Rescan'}</button>
        {isTauri && <button className="btn icon" title="Open folder in Explorer" onClick={() => act(() => revealPath(project.rootPath))}><LuFolderOpen /></button>}
      </div>
      {!project.rootAvailable && (
        <div className="alert bad" style={{ margin: '0 16px 8px' }}><LuTriangleAlert />
          <div className="grow">The project folder <span className="mono">{project.rootPath}</span> is not reachable — the drive may be disconnected, or the folder was moved or renamed. Your notes and history are safe; nothing has been marked as deleted.</div>
          {project.mode === 'linked' && <button className="btn sm" onClick={async () => { const p = await pickFolder('Select the new location of this project'); if (p) { await act(() => call('projects.relink', { id, path: p }), 'Folder re-linked'); await scan(false); } }}>Re-link folder…</button>}
          <button className="btn sm" onClick={() => scan(false)}>Retry</button>
        </div>
      )}
      <div className="tabs" role="tablist">
        {TABS.map((t, i) => (
          <button key={t.id} role="tab" className={cx(tab === t.id && 'on')} onClick={() => setTab(t.id)} title={`Alt+${i + 1}`}>
            {t.icon}{t.label}
            {t.id === 'files' && (project.changedFiles + project.pendingReviews > 0) && <span className="badge warn">{project.changedFiles + project.pendingReviews}</span>}
            {t.id === 'tasks' && project.openTasks > 0 && <span className="badge">{project.openTasks}</span>}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: tab === 'files' || tab === 'notes' ? 'hidden' : 'auto' }}>
        {tab === 'overview' && <Overview ctx={ctx} />}
        {tab === 'files' && <FilesTab ctx={ctx} />}
        {tab === 'notes' && <NotesTab ctx={ctx} />}
        {tab === 'tasks' && <TasksTab ctx={ctx} />}
        {tab === 'timeline' && <TimelineTab ctx={ctx} />}
        {tab === 'git' && <GitTab ctx={ctx} />}
      </div>
      {dropping && <div className="dropzone"><div className="card pad"><LuCircleCheck /> Drop to import into “{project.name}”</div></div>}
    </div>
  );
}
