import { Fragment, useCallback, useEffect, useMemo, useRef, useState, type ReactElement, type ReactNode } from 'react';
import { LuPlus, LuLayoutDashboard, LuSearch, LuSettings, LuFolder, LuRefreshCw, LuNotebookPen, LuListTodo, LuHistory, LuGitBranch, LuFiles, LuShieldCheck, LuCamera, LuGitCompareArrows, LuUndo2, LuDownload, LuCopy, LuFileSearch, LuSquareFunction } from 'react-icons/lu';
import { call, pickFolder, isTauri, revealPath } from '../ipc';
import { cx, ago, dateTime } from '../format';
import { KindIcon, useAction, useContextMenu, appContextMenu, useContextMenuOpen, useToast, type MenuItem } from '../components/common';
import { CompareDialog, RestoreDialog, REASON, type Version } from './Versions';
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
  /** a stored backup copy (never a live file) */
  backup?: Hit;
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
  const [syms, setSyms] = useState<any[]>([]);
  const [tick, setTick] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const menu = useContextMenu();
  const menuOpen = useContextMenuOpen();
  const actions = useFileActions();
  const terms = useMemo(() => q.trim().split(/\s+/).filter((t) => t.length > 0), [q]);
  const [compare, setCompare] = useState<Hit | null>(null);
  const [restore, setRestore] = useState<{ h: Hit; v: Version } | null>(null);
  const act = useAction();
  const toast = useToast();

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
    if (q.trim().length < 2) { setHits([]); setSyms([]); return; }
    let alive = true;
    const symOnly = q.trim().startsWith('@');
    const t = setTimeout(() => {
      if (!symOnly) call<Hit[]>('search', { query: q, limit: 25 }).then((h) => alive && setHits(h)).catch(() => alive && setHits([]));
      else setHits([]);
      // script functions (GSC/CSC): "@name" searches only functions
      call<any[]>('symbols.search', { query: q.trim(), limit: symOnly ? 30 : 6, projectId: route.view === 'project' && symOnly ? route.id : null }).then((r) => alive && setSyms(r)).catch(() => alive && setSyms([]));
    }, 120);
    return () => { alive = false; clearTimeout(t); };
  }, [q, tick]);

  const goFile = useCallback((h: Hit, action: 'reveal' | 'edit') => {
    onClose();
    nav.go({ view: 'project', id: h.projectId, tab: 'files', focus: { type: 'file', id: h.id, action, nonce: Date.now() } });
  }, [nav, onClose]);

  const goFolder = useCallback((projectId: string, dir: string) => {
    onClose();
    nav.go({ view: 'project', id: projectId, tab: 'files', focus: { type: 'folder', id: dir, nonce: Date.now() } });
  }, [nav, onClose]);
  const goHistory = useCallback((h: Hit) => {
    onClose();
    nav.go({ view: 'project', id: h.projectId, tab: 'files', focus: { type: 'file', id: h.fileId!, action: 'history', versionId: h.id, nonce: Date.now() } });
  }, [nav, onClose]);

  /** Project name and each folder are links that open that folder in the Files tab. */
  const crumbs = (projectId: string, projectName: string | undefined, dir: string) => {
    const segs = dir ? dir.split('/') : [];
    const link = (label: ReactNode, path: string, key: string) => (
      <span key={key} className="crumb" title={`Open ${path ? path + '/' : 'the project root'} in Files`}
        onClick={(e) => { e.stopPropagation(); if (e.button === 0) goFolder(projectId, path); }}
        onContextMenu={(e) => { e.stopPropagation(); appContextMenu(e.nativeEvent); }}>{label}</span>
    );
    return <>{link(<b>{projectName}</b>, '', 'root')}{segs.map((sg, i) => <Fragment key={i}>{' / '}{link(<Highlight text={sg} terms={terms} />, segs.slice(0, i + 1).join('/'), 'd' + i)}</Fragment>)}</>;
  };

  const items = useMemo<Item[]>(() => {
    const ql = q.toLowerCase();
    const cmds: Item[] = [
      { key: 'new', icon: <LuPlus />, label: 'New project…', run: onNewProject },
      { key: 'dash', icon: <LuLayoutDashboard />, label: 'Go to dashboard', run: () => nav.go({ view: 'dashboard' }) },
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
              <span className="ppath-dir">{crumbs(h.projectId, h.projectName, dir)}</span>
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
      if (h.type === 'backup') {
        const path = h.path || '';
        const dir = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
        return {
          key: 'backup' + h.id,
          icon: <LuShieldCheck />,
          label: <><span className="kindtag backup">backup copy</span><span className="fname-hl"><Highlight text={h.backupName || h.fileName || h.title} terms={terms} /></span></>,
          sub: <span className="ppath"><span className="ppath-dir">of {crumbs(h.projectId, h.projectName, dir)} / {h.fileName} · {h.at ? `${dateTime(h.at)} (${ago(h.at)})` : ''} · {REASON[h.reason || ''] || h.reason}</span></span>,
          title: `Stored copy of ${path} — not the live file`,
          snippet: h.description || undefined,
          backup: h,
          run: () => goHistory(h),
        };
      }
      if (h.type === 'snapshot') {
        return { key: 'snap' + h.id, icon: <LuCamera />, label: <><span className="kindtag snapshot">snapshot</span><Highlight text={h.title} terms={terms} /></>, sub: `${h.projectName || ''} · ${h.fileCount ?? '?'} files${h.at ? ' · ' + dateTime(h.at) : ''}`, snippet: h.snippet, run: () => { onClose(); openHit(nav, h); } };
      }
      return { key: h.type + h.id, icon: HIT_ICON[h.type], label: <Highlight text={h.title} terms={terms} />, sub: `${h.type} · ${h.projectName || ''}`, snippet: h.snippet, run: () => { onClose(); openHit(nav, h); } };
    });
    const fnTerm = q.trim().replace(/^@/, '');
    const symItems: Item[] = syms.map((d) => ({
      key: 'sym' + d.fileId + ':' + d.line,
      icon: <LuSquareFunction />,
      label: <><span className="kindtag">function</span><span className="mono"><Highlight text={d.signature} terms={[fnTerm]} /></span></>,
      sub: `${d.projectName} / ${d.path}:${d.line}`,
      run: () => { onClose(); nav.go({ view: 'project', id: d.projectId, tab: 'files', focus: { type: 'file', id: d.fileId, action: 'edit', line: d.line, nonce: Date.now() } }); },
    }));
    const all: Item = { key: 'allresults', icon: <LuSearch />, label: q.trim() ? `See all results for “${q.trim()}” (filter by type and project)` : 'Open full search', run: () => nav.go({ view: 'search', query: q }) };
    const list = q.trim().startsWith('@') ? symItems : [...local, ...remote, ...symItems];
    return [...list.slice(0, 59), ...(q.trim().length >= 2 && !q.trim().startsWith('@') ? [all] : [])];
  }, [q, hits, syms, nav, route, onNewProject, terms, goFile, goHistory, goFolder, onClose]);

  useEffect(() => setSel(0), [q]);
  useEffect(() => { (list.current?.querySelector('.item.on') as HTMLElement | null)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }, [sel]);

  const run = (i: Item) => { if (!i.file && !i.backup) onClose(); i.run(); };

  // Backup records get backup actions only: nothing here edits, renames or deletes a live file.
  const backupMenu = (h: Hit): MenuItem[] => [
    { label: 'Show in version history', icon: <LuHistory />, onClick: () => goHistory(h) },
    { label: 'Compare with current file', icon: <LuGitCompareArrows />, disabled: h.fileState !== 'present', onClick: () => setCompare(h) },
    { label: 'Restore this backup…', icon: <LuUndo2 />, onClick: async () => {
      const vs = await act(() => call<Version[]>('versions.list', { fileId: h.fileId }));
      const v = vs?.find((x) => x.id === h.id);
      if (v) setRestore({ h, v }); else if (vs) toast('This backup no longer exists.', 'error');
    } },
    { label: 'Save a copy as a file…', icon: <LuDownload />, onClick: async () => {
      const dest = await pickFolder('Save a copy of this backup to…');
      if (!dest) return;
      const r: any = await act(() => call('versions.export', { id: h.id, dest }));
      if (r) toast(`Saved ${r.path}`, 'success', isTauri ? { label: 'Show', onClick: () => revealPath(r.path) } : undefined);
    } },
    { sep: true, label: '' },
    { label: 'Reveal the original file in project', icon: <LuFileSearch />, disabled: h.fileState === 'deleted', onClick: () => goFile({ ...h, id: h.fileId!, type: 'file' }, 'reveal') },
    { label: 'Copy backup name', icon: <LuCopy />, onClick: () => navigator.clipboard?.writeText(h.backupName || h.fileName || '').then(() => toast('Copied', 'success'), () => toast('Clipboard not available', 'error')) },
  ];

  const openMenu = (pos: { clientX: number; clientY: number }, it: Item) => {
    if (it.backup) { menu({ ...pos, preventDefault() {} }, backupMenu(it.backup)); return; }
    if (!it.file) return;
    const h = it.file;
    const ref: FileRef = { id: h.id, name: h.name || h.title, path: h.path || h.title, absPath: h.absPath, projectId: h.projectId, projectName: h.projectName, state: h.state || 'present', editable: TEXT_EDITABLE(h.kind || '', h.ext || '', 0) };
    menu({ ...pos, preventDefault() {} }, actions.menuItems(ref, {
      onEdit: () => goFile(h, 'edit'),
      onOpen: () => goFile(h, 'reveal'),
      onReveal: () => goFile(h, 'reveal'),
      onHistory: () => { onClose(); nav.go({ view: 'project', id: h.projectId, tab: 'files', focus: { type: 'file', id: h.id, action: 'history', nonce: Date.now() } }); },
      onChanged: () => { setTick((t) => t + 1); nav.refreshProjects(); input.current?.focus(); },
    }));
  };

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal palette" role="dialog" aria-label="Search and commands">
        <input ref={input} value={q} placeholder="Search files, notes, tasks, projects, functions (@name) — or type a command…" onChange={(e) => setQ(e.target.value)}
          role="combobox" aria-expanded="true" aria-controls="pv-palette-list" aria-activedescendant={items[sel] ? `pv-item-${sel}` : undefined}
          onKeyDown={(e) => {
            if (e.key === 'Escape') { if (!menuOpen()) onClose(); }
            else if (e.key === 'ArrowDown') { e.preventDefault(); setSel((s) => Math.min(items.length - 1, s + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setSel((s) => Math.max(0, s - 1)); }
            else if (e.key === 'Enter' && items[sel]) { e.preventDefault(); run(items[sel]); }
            else if ((e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && (items[sel]?.file || items[sel]?.backup)) {
              e.preventDefault();
              const el = list.current?.querySelector('.item.on') as HTMLElement | null;
              const r = el?.getBoundingClientRect();
              if (r) openMenu({ clientX: r.left + 40, clientY: r.bottom }, items[sel]);
            }
          }} />
        <div className="items" id="pv-palette-list" role="listbox" ref={list}>
          {items.map((it, i) => (
            <div key={it.key} id={`pv-item-${i}`} role="option" aria-selected={i === sel} className={cx('item', i === sel && 'on', !!(it.file || it.backup) && 'fileitem', !!it.backup && 'backupitem')} title={it.title}
              onMouseMove={() => i !== sel && setSel(i)}
              onMouseDown={(e) => e.preventDefault() /* keep keyboard focus in the search box */}
              onClick={(e) => { if (e.button === 0) run(it); }}
              onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setSel(i); openMenu(e, it); }}>
              {it.icon}
              <div className="grow" style={{ minWidth: 0 }}>
                <div className="row" style={{ gap: 6 }}>{it.label}</div>
                {it.file || it.backup ? <div className="small dim">{it.sub}</div> : null}
                {it.snippet && <Snippet text={it.snippet} />}
              </div>
              {!it.file && !it.backup && it.sub && <span className="sub">{it.sub}</span>}
            </div>
          ))}
          {!items.length && <div className="muted small" style={{ padding: 10 }}>{q.trim().length < 2 ? 'Type to search.' : 'No matches.'}</div>}
        </div>
        <div className="palette-foot small dim"><kbd>↑</kbd><kbd>↓</kbd> move · <kbd>Enter</kbd> open · right-click or <kbd>Shift F10</kbd> for actions · <kbd>Esc</kbd> close</div>
      </div>
      {compare && <CompareDialog fileId={compare.fileId!} title={compare.fileName || ''} old={compare.id} new="current" onClose={() => { setCompare(null); input.current?.focus(); }} />}
      {restore && <RestoreDialog v={restore.v} fileName={restore.h.fileName || ''} present={restore.h.fileState === 'present'} onClose={() => { setRestore(null); input.current?.focus(); }} onDone={() => { setRestore(null); setTick((t) => t + 1); toast('Restored and verified', 'success'); input.current?.focus(); }} />}
    </div>
  );
}
