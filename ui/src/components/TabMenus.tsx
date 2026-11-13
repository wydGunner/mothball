import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { LuPlus, LuFolderOpen, LuCopy, LuChevronDown, LuLink } from 'react-icons/lu';
import { revealPath, isTauri, type Project } from '../ipc';
import { ProjectIcon, useToast } from './common';
import { cx } from '../format';

/** Closes on a click outside `ref` or Escape. */
function useDismiss(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) close(); };
    const key = (e: KeyboardEvent) => e.key === 'Escape' && close();
    window.addEventListener('mousedown', down);
    window.addEventListener('keydown', key);
    return () => { window.removeEventListener('mousedown', down); window.removeEventListener('keydown', key); };
  }, [open, close]);
  return ref;
}

/** "+" after the project tabs: open another project in a tab, or make a new one. */
export function NewTabButton({ projects, openIds, onOpen, onNew }: { projects: Project[]; openIds: string[]; onOpen: (id: string) => void; onNew: () => void }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);
  const close = () => { setOpen(false); setQ(''); setSel(0); };
  const ref = useDismiss(open, close);
  useEffect(() => { const on = () => setOpen(true); window.addEventListener('pv:newtab', on); return () => window.removeEventListener('pv:newtab', on); }, []);
  const list = useMemo(() => {
    const t = q.trim().toLowerCase();
    const hit = projects.filter((p) => !t || p.name.toLowerCase().includes(t) || (p.group || '').toLowerCase().includes(t));
    // not-yet-open first, then most recently opened
    return hit.sort((a, b) => Number(openIds.includes(a.id)) - Number(openIds.includes(b.id)) || (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0)).slice(0, 12);
  }, [projects, q, openIds]);
  const pick = (id: string) => { close(); onOpen(id); };
  return (
    <div className="newtab" ref={ref}>
      <button className={cx('newtab-btn', open && 'open')} title="Open a project in a new tab (Ctrl+T)" aria-haspopup="menu" aria-expanded={open} onClick={() => (open ? close() : setOpen(true))}><LuPlus size={15} /></button>
      {open && (
        <div className="dd-menu newtab-menu" role="menu">
          <input className="input" autoFocus placeholder="Open a project…" value={q} onChange={(e) => { setQ(e.target.value); setSel(0); }}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown') { e.preventDefault(); setSel((s) => Math.min(s + 1, list.length)); }
              if (e.key === 'ArrowUp') { e.preventDefault(); setSel((s) => Math.max(s - 1, 0)); }
              if (e.key === 'Enter') { e.preventDefault(); if (sel < list.length) pick(list[sel].id); else { close(); onNew(); } }
            }} />
          <div className="newtab-list">
            {list.map((p, i) => (
              <button key={p.id} role="menuitem" className={cx('dd-item', i === sel && 'sel')} onMouseEnter={() => setSel(i)} onClick={() => pick(p.id)}>
                <ProjectIcon category={p.category} icon={p.icon} color={p.color} size={16} />
                <span className="grow ell">{p.name}</span>
                {openIds.includes(p.id) ? <span className="dim small">open</span> : p.group ? <span className="dim small ell" style={{ maxWidth: 110 }}>{p.group}</span> : null}
              </button>
            ))}
            {!list.length && <div className="small dim" style={{ padding: '6px 10px' }}>No project matches “{q}”.</div>}
          </div>
          <div className="dd-sep" />
          <button role="menuitem" className={cx('dd-item', sel === list.length && 'sel')} onMouseEnter={() => setSel(list.length)} onClick={() => { close(); onNew(); }}><LuPlus size={14} /> <span className="grow">New project…</span><kbd>Ctrl N</kbd></button>
        </div>
      )}
    </div>
  );
}

/** The project's path under its name; click for folder actions. */
export function PathMenu({ path, linked, children }: { path: string; linked: boolean; children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useDismiss(open, () => setOpen(false));
  const toast = useToast();
  const item = (icon: ReactNode, label: string, fn: () => unknown) => (
    <button role="menuitem" className="dd-item" onClick={async () => { setOpen(false); try { await fn(); } catch (e: any) { toast(String(e?.message || e), 'error'); } }}>{icon}<span className="grow">{label}</span></button>
  );
  return (
    <div className="pathmenu" ref={ref}>
      <button className={cx('pathbtn mono', open && 'open')} onClick={() => setOpen(!open)} aria-haspopup="menu" aria-expanded={open} title={path}>
        {linked && <LuLink size={11} />}<span className="ell">{path}</span><LuChevronDown size={12} className="chev" />
      </button>
      {open && (
        <div className="dd-menu" role="menu" style={{ minWidth: 220, left: 0 }}>
          {isTauri && item(<LuFolderOpen size={14} />, 'Open in Explorer', () => revealPath(path))}
          {item(<LuCopy size={14} />, 'Copy path', async () => { await navigator.clipboard.writeText(path); toast('Path copied'); })}
          {children}
        </div>
      )}
    </div>
  );
}
