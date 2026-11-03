// The project type picker: a menu of types; types with subtypes (or your own
// types based on them) open a submenu to the side.
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as RKeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { LuChevronDown, LuChevronRight, LuCheck } from 'react-icons/lu';
import { ProjectIcon } from './common';
import { cx } from '../format';
import { splitKey, typeLabel, typeMenu, useTypes } from '../types';

const items = (el: HTMLElement | null) => Array.from(el?.querySelectorAll<HTMLButtonElement>('button[role^=menuitem]') || []);
function moveFocus(list: HTMLElement | null, dir: 1 | -1 | 'first' | 'last') {
  const all = items(list);
  if (!all.length) return;
  const i = all.indexOf(document.activeElement as HTMLButtonElement);
  const next = dir === 'first' ? 0 : dir === 'last' ? all.length - 1 : (i + dir + all.length) % all.length;
  all[next]?.focus();
}

export function TypeSelect({ value, onChange, keep }: { value: string; onChange: (key: string) => void; keep?: string }) {
  useTypes();
  const [open, setOpen] = useState(false);
  const [sub, setSub] = useState<string | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number; width: number; maxH: number } | null>(null);
  const [subPos, setSubPos] = useState<{ left: number; top: number } | null>(null);
  const focusSub = useRef(false);
  const btn = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const flyout = useRef<HTMLDivElement>(null);
  const rows = typeMenu(keep ?? value);
  const cur = splitKey(value).category;

  const place = () => {
    const r = btn.current!.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 12;
    const up = below < 240 && r.top > below;
    const maxH = Math.min(420, (up ? r.top : below) - 8);
    setPos({ left: r.left, top: up ? Math.max(8, r.top - 6 - Math.min(maxH, rows.length * 34 + 10)) : r.bottom + 6, width: Math.max(r.width, 240), maxH });
  };
  const close = (refocus = true) => { setOpen(false); setSub(null); if (refocus) btn.current?.focus(); };
  const openMenu = () => { place(); setOpen(true); setSub(null); };
  const choose = (k: string) => { onChange(k); close(); };

  // open a submenu beside its row, flipping left if there's no room on the right
  const openSub = (key: string, row: HTMLElement, focusFirst = false) => {
    const r = row.getBoundingClientRect();
    setSub(key);
    setSubPos({ left: r.right + 4, top: r.top - 5 });
    focusSub.current = focusFirst; // focused as soon as the submenu renders (see below), so a quick Enter lands in it
  };
  useLayoutEffect(() => {
    const f = flyout.current;
    if (!f || !subPos) return;
    if (focusSub.current) { focusSub.current = false; moveFocus(f, 'first'); }
    const r = f.getBoundingClientRect();
    let { left, top } = subPos;
    if (r.right > window.innerWidth - 8) left = (menu.current?.getBoundingClientRect().left ?? left) - r.width - 4;
    if (r.bottom > window.innerHeight - 8) top = Math.max(8, window.innerHeight - 8 - r.height);
    if (left !== subPos.left || top !== subPos.top) setSubPos({ left, top });
  }, [subPos, sub]);

  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!btn.current?.contains(t) && !menu.current?.contains(t) && !flyout.current?.contains(t)) close(false);
    };
    const scroll = (e: Event) => { if (!menu.current?.contains(e.target as Node) && !flyout.current?.contains(e.target as Node)) close(false); };
    const resize = () => close(false);
    // while open, Escape belongs to the menu wherever focus is (never closes a dialog behind it)
    const esc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || flyout.current?.contains(document.activeElement)) return;
      e.preventDefault(); e.stopImmediatePropagation(); close();
    };
    window.addEventListener('keydown', esc, true);
    window.addEventListener('mousedown', outside);
    window.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', resize);
    setTimeout(() => (menu.current?.querySelector<HTMLButtonElement>('button.tm-item.on') || items(menu.current)[0])?.focus(), 0); // start at the current type
    return () => { window.removeEventListener('keydown', esc, true); window.removeEventListener('mousedown', outside); window.removeEventListener('scroll', scroll, true); window.removeEventListener('resize', resize); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const onMenuKey = (e: RKeyboardEvent) => {
    const row = document.activeElement as HTMLElement;
    if (e.key === 'ArrowDown') { e.preventDefault(); moveFocus(menu.current, 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); moveFocus(menu.current, -1); }
    else if (e.key === 'Home') { e.preventDefault(); moveFocus(menu.current, 'first'); }
    else if (e.key === 'End') { e.preventDefault(); moveFocus(menu.current, 'last'); }
    else if (e.key === 'ArrowRight' && row?.dataset.parent) { e.preventDefault(); openSub(row.dataset.parent, row, true); }
    else if (e.key === 'Escape' || e.key === 'Tab') { e.preventDefault(); e.stopPropagation(); close(); } // only the menu, not a dialog behind it
  };
  const onSubKey = (e: RKeyboardEvent) => {
    e.stopPropagation();
    if (e.key === 'ArrowDown') { e.preventDefault(); moveFocus(flyout.current, 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); moveFocus(flyout.current, -1); }
    else if (e.key === 'ArrowLeft' || e.key === 'Escape') {
      e.preventDefault();
      const parent = sub;
      setSub(null);
      menu.current?.querySelector<HTMLButtonElement>(`button[data-parent="${parent}"]`)?.focus();
    } else if (e.key === 'Tab') { e.preventDefault(); close(); }
  };

  const label = typeLabel(value);
  const subRow = rows.find((r) => r.key === sub);
  return (
    <>
      <button ref={btn} type="button" className={cx('input typesel', open && 'open')} aria-haspopup="menu" aria-expanded={open} aria-label={`Project type: ${label}`}
        onClick={() => (open ? close() : openMenu())}
        onKeyDown={(e) => { if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); openMenu(); } }}>
        <ProjectIcon category={cur} size={18} /><span className="grow">{label}</span><LuChevronDown size={14} className="dim" />
      </button>
      {open && pos && createPortal(
        <div ref={menu} className="tmenu" role="menu" aria-label="Project type" style={{ left: pos.left, top: pos.top, width: pos.width, maxHeight: pos.maxH }} onKeyDown={onMenuKey}>
          {rows.map((r) => {
            const here = r.children ? r.children.some(([k]) => k === value) : r.key === value;
            return r.children ? (
              <button key={r.key} type="button" role="menuitem" aria-haspopup="menu" aria-expanded={sub === r.key} data-parent={r.key}
                className={cx('tm-item', here && 'on', sub === r.key && 'sub-open')}
                onMouseEnter={(e) => openSub(r.key, e.currentTarget)} onClick={(e) => openSub(r.key, e.currentTarget, true)}>
                <ProjectIcon category={r.key} size={18} /><span className="grow">{r.label}</span><LuChevronRight size={14} className="dim" />
              </button>
            ) : (
              <button key={r.key} type="button" role="menuitemradio" aria-checked={here} className={cx('tm-item', here && 'on')}
                onMouseEnter={() => setSub(null)} onClick={() => choose(r.key)}>
                <ProjectIcon category={r.key} size={18} /><span className="grow">{r.label}</span>{here && <LuCheck size={14} className="tm-check" />}
              </button>
            );
          })}
        </div>, document.body)}
      {open && subRow?.children && subPos && createPortal(
        <div ref={flyout} className="tmenu tm-sub" role="menu" aria-label={subRow.label} style={{ left: subPos.left, top: subPos.top }} onKeyDown={onSubKey}>
          {subRow.children.map(([k, l]) => (
            <button key={k} type="button" role="menuitemradio" aria-checked={k === value} className={cx('tm-item', k === value && 'on')} onClick={() => choose(k)}>
              <span className="grow">{l}</span>{k === value && <LuCheck size={14} className="tm-check" />}
            </button>
          ))}
        </div>, document.body)}
    </>
  );
}
