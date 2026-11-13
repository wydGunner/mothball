import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { LuLayoutDashboard, LuFiles, LuNotebookPen, LuListTodo, LuHistory, LuGitBranch, LuRefreshCw, LuTriangleAlert, LuStar, LuChevronDown, LuChevronRight, LuCircleCheck, LuShieldCheck } from 'react-icons/lu';
import { call, pickFolder, onFileDrop, type Project, type ScanReport } from '../ipc';
import { PathMenu } from '../components/TabMenus';
import { Empty, ProjectIcon, StatusPill, useAction, useToast, useConfirm } from '../components/common';
import { ago, cx } from '../format';
import { FirstTips } from './Welcome';
import { DeployButtons } from './Deploy';
import { TimerChip } from './TimeCard';
import type { Focus, Nav } from '../main';
import { Overview } from './Overview';
import { FilesTab } from './Files';
import { NotesTab } from './Notes';
import { TasksTab } from './Tasks';
import { TimelineTab } from './Timeline';
import { GitTab } from './Git';
import { HistoryTab, CheckpointButton } from './History';
import { ImportDialog } from './Import';

export type Tab = 'overview' | 'files' | 'notes' | 'tasks' | 'timeline' | 'backups' | 'git';
const TABS: { id: Tab; label: string; icon: ReactElement }[] = [
  { id: 'overview', label: 'Overview', icon: <LuLayoutDashboard /> },
  { id: 'files', label: 'Files', icon: <LuFiles /> },
  { id: 'notes', label: 'Notes', icon: <LuNotebookPen /> },
  { id: 'tasks', label: 'Tasks & bugs', icon: <LuListTodo /> },
  { id: 'timeline', label: 'Timeline', icon: <LuHistory /> },
  { id: 'backups', label: 'Backups', icon: <LuShieldCheck /> },
  { id: 'git', label: 'Git', icon: <LuGitBranch /> },
];

const AUTO_SCAN_MS = 60_000;

export interface ProjectCtx {
  keepFilesAlive: (b: boolean) => void;
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
  focus?: Focus;
}

/** `active`: this is the visible tab. Hidden tabs stay mounted (open editors and
 *  unsaved edits survive switching) but don't react to keys, drops or focus. */
export function ProjectView({ nav, id, tab: initialTab, focus: initialFocus, active = true }: { nav: Nav; id: string; tab?: Tab; focus?: Focus; active?: boolean }) {
  const activeRef = useRef(active);
  activeRef.current = active;
  const [project, setProject] = useState<Project | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>(initialTab || 'overview');
  // the Files tab stays mounted while it has open editors, so unsaved edits survive tab switches
  const [filesVisited, setFilesVisited] = useState(false);
  const [timerTick, setTimerTick] = useState(0);
  useEffect(() => { const f = () => setTimerTick((t) => t + 1); window.addEventListener('pv:timer', f); return () => window.removeEventListener('pv:timer', f); }, []);
  const [focus, setFocus] = useState(initialFocus);
  const [scanning, setScanning] = useState(false);
  const [scanTick, setScanTick] = useState(0);
  const [dropping, setDropping] = useState(false);
  const [fileGroup, setFileGroup] = useState<string | undefined>(undefined);
  const [dropImport, setDropImport] = useState<string[] | null>(null);
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
        toast(changes || r.needsReview ? `Changes detected: ${parts.join(', ')}` : `Up to date (${r.total} ${r.total === 1 ? 'file' : 'files'})`, changes ? 'info' : 'success');
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
  // The background watcher records changes as they happen; when it is
  // switched off, fall back to a periodic rescan of the open project.
  useEffect(() => {
    const onFocus = () => { if (activeRef.current) scan(true); };
    window.addEventListener('focus', onFocus);
    const onChanged = () => { setScanTick((t) => t + 1); reload(); };
    window.addEventListener('pv:changed', onChanged);
    let t: ReturnType<typeof setInterval> | undefined;
    call<{ enabled: boolean }>('watch.settings').then((w) => {
      if (!w.enabled) t = setInterval(() => { if (document.visibilityState === 'visible' && activeRef.current) scan(true); }, AUTO_SCAN_MS);
    }).catch(() => {});
    return () => { window.removeEventListener('focus', onFocus); window.removeEventListener('pv:changed', onChanged); if (t) clearInterval(t); };
  }, [scan, reload]);

  // coming back to a tab: catch up with changes made meanwhile
  const wasActive = useRef(active);
  useEffect(() => { if (active && !wasActive.current) scan(true); wasActive.current = active; }, [active, scan]);

  // OS drag-and-drop import (desktop app)
  useEffect(() => {
    let off: (() => void) | undefined;
    onFileDrop(async (paths) => {
      if (!paths.length || !activeRef.current) return;
      // always confirm: shows the destination project/folder and what will happen
      setDropImport(paths);
    }, (over) => { if (activeRef.current) setDropping(over); }).then((f) => { off = f; });
    return () => off?.();
  }, [id, project?.mode, act, confirm, toast, reload]);

  // header folded to one line (all projects, remembered)
  const [compact, setCompactRaw] = useState(() => { try { return localStorage.getItem('pv.headerCompact') === '1'; } catch { return false; } });
  const setCompact = (c: boolean) => { setCompactRaw(c); try { localStorage.setItem('pv.headerCompact', c ? '1' : '0'); } catch { /* ignore */ } window.dispatchEvent(new CustomEvent('pv:header-compact', { detail: c })); };
  useEffect(() => { const on = (e: Event) => setCompactRaw((e as CustomEvent).detail); window.addEventListener('pv:header-compact', on); return () => window.removeEventListener('pv:header-compact', on); }, []);
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (!activeRef.current) return;
      if (e.altKey && /^[1-7]$/.test(e.key)) { e.preventDefault(); setTab(TABS[Number(e.key) - 1].id); }
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
    keepFilesAlive: setFilesVisited,
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className={cx('phead', compact && 'compact')}>
        <button className="phead-fold" title={compact ? 'Show the full header' : 'Fold the header to one line'} aria-expanded={!compact} onClick={() => setCompact(!compact)}>{compact ? <LuChevronRight size={14} /> : <LuChevronDown size={14} />}</button>
        <ProjectIcon category={project.category} icon={project.icon} color={project.color} size={compact ? 20 : 34} />
        <div className="grow" style={{ minWidth: 0 }}>
          <div className="row"><h1 style={{ fontSize: compact ? 15 : 20 }}>{project.name}</h1>{project.status !== 'Active' && <StatusPill status={project.status} />}
            <button className={cx('star', project.favorite && 'on')} title="Favorite" onClick={async () => { await act(() => call('projects.update', { id, patch: { favorite: !project.favorite } })); reload(); nav.refreshProjects(); }}><LuStar fill={project.favorite ? 'currentColor' : 'none'} /></button>
          </div>
          {!compact && <PathMenu path={project.rootPath} linked={project.mode === 'linked'} />}
        </div>
        <span className="small dim">{project.lastScanAt ? `Checked ${ago(project.lastScanAt)}` : ''}</span>
        <TimerChip projectId={project.id} tick={timerTick} />
        <DeployButtons project={project} />
        {project.rootAvailable && <CheckpointButton projectId={project.id} rootPath={project.rootPath} scanTick={scanTick} onRestored={() => scan(true)} />}
        <button className="btn" onClick={() => scan(false)} disabled={scanning} title="Re-scan for external changes (F5)"><LuRefreshCw className={scanning ? 'spin' : ''} /> {scanning ? 'Scanning…' : 'Rescan'}</button>
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
            {t.id === 'files' && (project.missingFiles + project.pendingReviews + (project.staleDocs || 0) > 0) && <span className="badge warn" title="Missing files, file matches to confirm, or notes that may be out of date">{project.missingFiles + project.pendingReviews + (project.staleDocs || 0)}</span>}
            {t.id === 'tasks' && project.openTasks > 0 && <span className="badge">{project.openTasks}</span>}
          </button>
        ))}
      </div>
      <FirstTips />
      <div style={{ flex: 1, minHeight: 0, overflow: tab === 'files' || tab === 'notes' ? 'hidden' : 'auto' }}>
        {tab === 'overview' && <Overview ctx={ctx} />}
        {/* stays mounted while it has open editors */}
        {(tab === 'files' || filesVisited) && <div style={{ display: tab === 'files' ? 'contents' : 'none' }}><FilesTab ctx={ctx} /></div>}
        {tab === 'notes' && <NotesTab ctx={ctx} />}
        {tab === 'tasks' && <TasksTab ctx={ctx} />}
        {tab === 'timeline' && <TimelineTab ctx={ctx} />}
        {tab === 'backups' && <HistoryTab ctx={ctx} />}
        {tab === 'git' && <GitTab ctx={ctx} />}
      </div>
      {dropImport && <ImportDialog projectId={id} paths={dropImport} initialDir="" onClose={() => setDropImport(null)} onDone={() => { setScanTick((t) => t + 1); reload(); nav.refreshProjects(); }} />}
      {dropping && <div className="dropzone"><div className="card pad"><LuCircleCheck /> Drop to import into “{project.name}” — you'll confirm the folder next</div></div>}
    </div>
  );
}
