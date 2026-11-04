import { useEffect, useRef, useState, type ReactElement } from 'react';
import { LuSearch, LuFile, LuNotebookPen, LuListTodo, LuBug, LuFolder, LuBookOpen, LuShieldCheck, LuCamera } from 'react-icons/lu';
import { call } from '../ipc';
import { Empty } from '../components/common';
import { cx, dateTime } from '../format';
import type { Nav } from '../main';

export interface Hit {
  type: string; id: string; projectId: string; projectName?: string; title: string; snippet: string; path?: string; state?: string; name?: string; ext?: string; kind?: string; absPath?: string;
  /** backup / snapshot results */
  fileId?: string; fileName?: string; backupName?: string; description?: string; reason?: string; at?: number; fileState?: string; fileCount?: number;
}

export const HIT_ICON: Record<string, ReactElement> = { project: <LuFolder />, file: <LuFile />, note: <LuNotebookPen />, task: <LuListTodo />, bug: <LuBug color="var(--accent)" />, journal: <LuBookOpen />, backup: <LuShieldCheck />, snapshot: <LuCamera /> };
const TYPES = [['', 'Everything'], ['project', 'Projects'], ['file', 'Files'], ['note', 'Notes'], ['task', 'Tasks'], ['bug', 'Bugs'], ['journal', 'Journal'], ['backup', 'Backups'], ['snapshot', 'Snapshots']] as const;

export function openHit(nav: Nav, h: Hit) {
  if (h.type === 'project') return nav.go({ view: 'project', id: h.id });
  if (h.type === 'backup') return nav.go({ view: 'project', id: h.projectId, tab: 'files', focus: { type: 'file', id: h.fileId!, action: 'history', versionId: h.id, nonce: Date.now() } });
  if (h.type === 'snapshot') return nav.go({ view: 'project', id: h.projectId, tab: 'backups' });
  const focusType = h.type === 'file' ? 'file' : h.type === 'note' ? 'note' : h.type === 'journal' ? undefined : 'task';
  const tab = h.type === 'file' ? 'files' : h.type === 'note' ? 'notes' : h.type === 'journal' ? 'timeline' : 'tasks';
  nav.go({ view: 'project', id: h.projectId, tab, focus: focusType ? { type: focusType, id: h.id } : undefined });
}

/** Renders FTS snippet markers (\u0001…\u0002) as <mark>, safely. */
export function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0001[^\u0002]*\u0002)/);
  return <span className="snippet">{parts.map((p, i) => p.startsWith('\u0001') ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p}</span>)}</span>;
}

export function SearchView({ nav, initial }: { nav: Nav; initial?: string }) {
  const [q, setQ] = useState(initial || '');
  const [type, setType] = useState('');
  const [project, setProject] = useState('');
  const [hits, setHits] = useState<Hit[] | null>(null);
  const [err, setErr] = useState('');
  const [ms, setMs] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  useEffect(() => {
    if (!q.trim()) { setHits(null); return; }
    const t = setTimeout(async () => {
      const t0 = performance.now();
      try { setHits(await call<Hit[]>('search', { query: q, types: type ? [type] : [], projectId: project || null, limit: 200 })); setErr(''); } catch (e: any) { setErr(e.message); }
      setMs(Math.round(performance.now() - t0));
    }, 150);
    return () => clearTimeout(t);
  }, [q, type, project]);

  return (
    <div className="page narrow" style={{ maxWidth: 1100 }}>
      <div className="row"><LuSearch size={18} /><input ref={input} className="input" style={{ fontSize: 15, padding: '9px 12px' }} placeholder="Search projects, file names & paths, file documentation, notes, tasks, bugs, journal…" value={q} onChange={(e) => setQ(e.target.value)} /></div>
      <div className="toolbar">
        <div className="seg">{TYPES.map(([k, l]) => <button key={k} className={cx(type === k && 'on')} onClick={() => setType(k)}>{l}</button>)}</div>
        <select className="input" style={{ width: 220 }} value={project} onChange={(e) => setProject(e.target.value)} aria-label="Project"><option value="">All projects</option>{nav.projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
        {hits && <span className="small dim right">{hits.length} results · {ms} ms</span>}
      </div>
      {err && <div className="alert bad">{err}</div>}
      {hits === null ? <Empty icon={<LuSearch />} title="Search everything">Words match as prefixes: <code>weap tun</code> finds “weapon_tuner.gsc”. Results are ranked by relevance; titles weigh more than body text.</Empty>
        : !hits.length ? <Empty title="No results" />
        : hits.map((h) => (
          <div key={h.type + h.id} className="card" style={{ padding: '10px 14px', marginBottom: 6, cursor: 'pointer' }} onClick={() => openHit(nav, h)}>
            <div className="row">{HIT_ICON[h.type]}{(h.type === 'backup' || h.type === 'snapshot') && <span className={`kindtag ${h.type}`}>{h.type === 'backup' ? 'backup copy' : 'snapshot'}</span>}<b className={h.type === 'file' || h.type === 'backup' ? 'mono' : ''}>{h.title}</b>{h.state === 'missing' && <span className="badge bad">missing</span>}<span className="small dim right">{h.type === 'backup' ? 'stored copy, not the live file' : h.type} · {h.projectName}</span></div>
            {h.type === 'backup' && <div className="small dim">Backup of <span className="mono">{h.path}</span>{h.at ? ` · ${dateTime(h.at)}` : ''}</div>}
            {h.type === 'snapshot' && <div className="small dim">Project snapshot{h.at ? ` · ${dateTime(h.at)}` : ''}{h.fileCount != null ? ` · ${h.fileCount} ${h.fileCount === 1 ? 'file' : 'files'}` : ''}</div>}
            {h.path && h.type !== 'backup' && <div className="small dim mono">{h.path}</div>}
            <Snippet text={h.snippet} />
          </div>
        ))}
    </div>
  );
}
