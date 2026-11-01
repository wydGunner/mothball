import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react';
import { LuPlus, LuLayoutDashboard, LuSearch, LuSettings, LuFolder, LuRefreshCw, LuNotebookPen, LuListTodo, LuHistory, LuGitBranch, LuFiles } from 'react-icons/lu';
import { call } from '../ipc';
import { cx } from '../format';
import type { Nav, Route } from '../main';
import { HIT_ICON, openHit, Snippet, type Hit } from './Search';

interface Item { key: string; icon: ReactElement; label: string; sub?: string; snippet?: string; run: () => void }

export function CommandPalette({ nav, route, onClose, onNewProject }: { nav: Nav; route: Route; onClose: () => void; onNewProject: () => void }) {
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);
  const [hits, setHits] = useState<Hit[]>([]);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.focus(), []);
  useEffect(() => {
    if (q.trim().length < 2) { setHits([]); return; }
    const t = setTimeout(() => call<Hit[]>('search', { query: q, limit: 12 }).then(setHits).catch(() => setHits([])), 120);
    return () => clearTimeout(t);
  }, [q]);

  const items = useMemo<Item[]>(() => {
    const ql = q.toLowerCase();
    const cmds: Item[] = [
      { key: 'new', icon: <LuPlus />, label: 'New project…', run: onNewProject },
      { key: 'dash', icon: <LuLayoutDashboard />, label: 'Go to dashboard', run: () => nav.go({ view: 'dashboard' }) },
      { key: 'search', icon: <LuSearch />, label: 'Open full search', run: () => nav.go({ view: 'search', query: q }) },
      { key: 'settings', icon: <LuSettings />, label: 'Settings & backup', run: () => nav.go({ view: 'settings' }) },
    ];
    if (route.view === 'project') {
      const id = route.id;
      cmds.push(
        { key: 't-files', icon: <LuFiles />, label: 'Project: Files', run: () => nav.go({ view: 'project', id, tab: 'files' }) },
        { key: 't-notes', icon: <LuNotebookPen />, label: 'Project: Notes', run: () => nav.go({ view: 'project', id, tab: 'notes' }) },
        { key: 't-tasks', icon: <LuListTodo />, label: 'Project: Tasks & bugs', run: () => nav.go({ view: 'project', id, tab: 'tasks' }) },
        { key: 't-tl', icon: <LuHistory />, label: 'Project: Timeline', run: () => nav.go({ view: 'project', id, tab: 'timeline' }) },
        { key: 't-git', icon: <LuGitBranch />, label: 'Project: Git', run: () => nav.go({ view: 'project', id, tab: 'git' }) },
        { key: 'rescan', icon: <LuRefreshCw />, label: 'Project: Rescan files (F5)', run: () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'F5' })) },
      );
    }
    const projects: Item[] = nav.projects.map((p) => ({ key: 'p' + p.id, icon: <LuFolder />, label: p.name, sub: `${p.status} · project`, run: () => nav.go({ view: 'project', id: p.id }) }));
    const local = [...cmds, ...projects].filter((i) => !ql || i.label.toLowerCase().includes(ql));
    const remote: Item[] = hits.filter((h) => h.type !== 'project').map((h) => ({ key: h.type + h.id, icon: HIT_ICON[h.type], label: h.title, sub: `${h.type} · ${h.projectName || ''}`, snippet: h.snippet, run: () => openHit(nav, h) }));
    return [...local, ...remote].slice(0, 40);
  }, [q, hits, nav, route, onNewProject]);

  useEffect(() => setSel(0), [q]);
  const run = (i: Item) => { onClose(); i.run(); };
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal palette" role="dialog" aria-label="Command palette">
        <input ref={ref} value={q} placeholder="Type a command, project, file, note or task…" onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') onClose();
            else if (e.key === 'ArrowDown') { e.preventDefault(); setSel((s) => Math.min(items.length - 1, s + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setSel((s) => Math.max(0, s - 1)); }
            else if (e.key === 'Enter' && items[sel]) run(items[sel]);
          }} />
        <div className="items">
          {items.map((it, i) => (
            <div key={it.key} className={cx('item', i === sel && 'on')} onMouseEnter={() => setSel(i)} onClick={() => run(it)}>
              {it.icon}<div style={{ minWidth: 0 }}><div>{it.label}</div>{it.snippet && <Snippet text={it.snippet} />}</div>{it.sub && <span className="sub">{it.sub}</span>}
            </div>
          ))}
          {!items.length && <div className="muted small" style={{ padding: 10 }}>No matches.</div>}
        </div>
      </div>
    </div>
  );
}
