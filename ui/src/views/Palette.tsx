import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { LuPlus, LuLayoutDashboard, LuSearch, LuSettings, LuFolder, LuRefreshCw, LuNotebookPen, LuListTodo, LuHistory, LuGitBranch, LuFiles } from 'react-icons/lu';
import { call } from '../ipc';
import { cx } from '../format';
import { KindIcon, useContextMenu, useContextMenuOpen } from '../components/common';
import { useFileActions, TEXT_EDITABLE, type FileRef } from '../fileActions';
import type { Nav, Route } from '../main';
import { HIT_ICON, openHit, Snippet, type Hit } from './Search';

interface Item {
  key: string;
  icon: ReactElement;
  label: ReactNode;
  sub?: ReactNode;
  snippet?: string;
  title?: string;
  file?: Hit;
  run: () => void;
}

/** Highlights each search term (prefix match, case-insensitive) inside `text`. */
export function Highlight({ text, terms }: { text: string; terms: string[] }) {
  if (!terms.length) return <>{text}</>;
  const esc = terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
  const parts = text.split(new RegExp(`(${esc})`, 'ig'));
  return <>{parts.map((p, i) => (i % 2 ? <mark key={i}>{p}</mark> : <Fragment key={i}>{p}</Fragment>))}</>;
}

export function CommandPalette({ nav, route, onClose, onNewProject }: { nav: Nav; route: Route; onClose: () => void; onNewProject: () => void }) {
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);
  const [hits, setHits] = useState<Hit[]>([]);
  const [tick, setTick] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const menu = useContextMenu();
  const menuOpen = useContextMenuOpen();
  const actions = useFileActions();
  const terms = useMemo(() => q.trim().split(/\s+/).filter((t) => t.length > 0), [q]);

  useEffect(() => input.current?.focus(), []);
  // Escape closes the palette unless a context menu or a dialog (rename/delete) is on top;
  // focus returns to the search box after menu actions so the keyboard keeps working.
  useEffect(() => {
    const otherDialog = () => document.querySelectorAll('.modal').length > 1;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && document.activeElement !== input.current && !menuOpen() && !otherDialog()) onClose(); };
    const refocus = () => setTimeout(() => { if (!menuOpen() && !otherDialog() && (document.activeElement === document.body || !document.activeElement)) input.current?.focus(); }, 0);
    window.addEventListener('keydown', onKey);
    window.addEventListener('click', refocus);
    return () => { window.removeEventListener('keydown', onKey); window.removeEventListener('click', refocus); };
  }, [menuOpen, onClose]);
  useEffect(() => {
    if (q.trim().length < 2) { setHits([]); return; }
    let alive = true;
    const t = setTimeout(() => call<Hit[]>('search', { query: q, limit: 25 }).then((h) => alive && setHits(h)).catch(() => alive && setHits([])), 120);
    return () => { alive = false; clearTimeout(t); };
  }, [q, tick]);

  const goFile = useCallback((h: Hit, action: 'reveal' | 'edit') => {
    onClose();
    nav.go({ view: 'project', id: h.projectId, tab: 'files', focus: { type: 'file', id: h.id, action, nonce: Date.now() } });
  }, [nav, onClose]);

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
    const local: Item[] = [...cmds, ...nav.projects.map((p) => ({ key: 'p' + p.id, icon: <LuFolder />, label: p.name, sub: `${p.status} · project`, run: () => nav.go({ view: 'project', id: p.id }) }))]
      .filter((i) => !ql || String(i.label).toLowerCase().includes(ql))
      .map((i) => ({ ...i, label: <Highlight text={String(i.label)} terms={terms} /> }));
    const remote: Item[] = hits.filter((h) => h.type !== 'project').map((h) => {
      if (h.type === 'file') {
        const path = h.path || h.title;
        const dir = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
        const name = h.name || path.split('/').pop() || h.title;
        return {
          key: 'file' + h.id,
          icon: <KindIcon kind={h.kind || 'other'} />,
          label: <><span className="fname-hl"><Highlight text={name} terms={terms} /></span>{h.state === 'missing' && <span className="badge bad">missing</span>}</>,
          sub: (
            <span className="ppath">
              <span className="ppath-dir"><b>{h.projectName}</b>{dir && <> / <Highlight text={dir.split('/').join(' / ')} terms={terms} /></>}</span>
              <span className="ppath-name">{'\u00a0/\u00a0'}{name}</span>
            </span>
          ),
          title: h.absPath || path,
          // show matched note/description text, but not when it's just the path again
          snippet: (() => { const plain = h.snippet.replace(/[\u0001\u0002…]/g, '').trim(); return plain && !path.includes(plain) && !plain.includes(path) ? h.snippet : undefined; })(),
          file: h,
          run: () => goFile(h, 'reveal'),
        };
      }
      return { key: h.type + h.id, icon: HIT_ICON[h.type], label: <Highlight text={h.title} terms={terms} />, sub: `${h.type} · ${h.projectName || ''}`, snippet: h.snippet, run: () => { onClose(); openHit(nav, h); } };
    });
    return [...local, ...remote].slice(0, 50);
  }, [q, hits, nav, route, onNewProject, terms, goFile, onClose]);

  useEffect(() => setSel(0), [q]);
  useEffect(() => { (list.current?.querySelector('.item.on') as HTMLElement | null)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }, [sel]);

  const run = (i: Item) => { if (!i.file) onClose(); i.run(); };

  const openMenu = (pos: { clientX: number; clientY: number }, it: Item) => {
    if (!it.file) return;
    const h = it.file;
    const ref: FileRef = { id: h.id, name: h.name || h.title, path: h.path || h.title, absPath: h.absPath, projectId: h.projectId, projectName: h.projectName, state: h.state || 'present', editable: TEXT_EDITABLE(h.kind || '', h.ext || '', 0) };
    menu({ ...pos, preventDefault() {} }, actions.menuItems(ref, {
      onEdit: () => goFile(h, 'edit'),
      onOpen: () => goFile(h, 'reveal'),
      onReveal: () => goFile(h, 'reveal'),
      onChanged: () => { setTick((t) => t + 1); nav.refreshProjects(); input.current?.focus(); },
    }));
  };

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal palette" role="dialog" aria-label="Search and commands">
        <input ref={input} value={q} placeholder="Search files, notes, tasks, projects — or type a command…" onChange={(e) => setQ(e.target.value)}
          role="combobox" aria-expanded="true" aria-controls="pv-palette-list" aria-activedescendant={items[sel] ? `pv-item-${sel}` : undefined}
          onKeyDown={(e) => {
            if (e.key === 'Escape') { if (!menuOpen()) onClose(); }
            else if (e.key === 'ArrowDown') { e.preventDefault(); setSel((s) => Math.min(items.length - 1, s + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setSel((s) => Math.max(0, s - 1)); }
            else if (e.key === 'Enter' && items[sel]) { e.preventDefault(); run(items[sel]); }
            else if ((e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && items[sel]?.file) {
              e.preventDefault();
              const el = list.current?.querySelector('.item.on') as HTMLElement | null;
              const r = el?.getBoundingClientRect();
              if (r) openMenu({ clientX: r.left + 40, clientY: r.bottom }, items[sel]);
            }
          }} />
        <div className="items" id="pv-palette-list" role="listbox" ref={list}>
          {items.map((it, i) => (
            <div key={it.key} id={`pv-item-${i}`} role="option" aria-selected={i === sel} className={cx('item', i === sel && 'on', !!it.file && 'fileitem')} title={it.title}
              onMouseMove={() => i !== sel && setSel(i)}
              onClick={(e) => { if (e.button === 0) run(it); }}
              onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setSel(i); openMenu(e, it); }}>
              {it.icon}
              <div className="grow" style={{ minWidth: 0 }}>
                <div className="row" style={{ gap: 6 }}>{it.label}</div>
                {it.file ? <div className="small dim">{it.sub}</div> : null}
                {it.snippet && <Snippet text={it.snippet} />}
              </div>
              {!it.file && it.sub && <span className="sub">{it.sub}</span>}
            </div>
          ))}
          {!items.length && <div className="muted small" style={{ padding: 10 }}>{q.trim().length < 2 ? 'Type to search.' : 'No matches.'}</div>}
        </div>
        <div className="palette-foot small dim"><kbd>↑</kbd><kbd>↓</kbd> move · <kbd>Enter</kbd> open · right-click or <kbd>Shift F10</kbd> for file actions · <kbd>Esc</kbd> close</div>
      </div>
    </div>
  );
}
