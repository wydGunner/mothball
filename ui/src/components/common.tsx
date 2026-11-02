import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  LuFileCode, LuFileText, LuSettings2, LuImage, LuBox, LuShapes, LuPrinter, LuArchive, LuGamepad2, LuMusic, LuVideo,
  LuCpu, LuBinary, LuFile, LuX, LuCircleAlert, LuCircleCheck, LuInfo, LuFolderGit2, LuCode, LuWrench, LuPalette, LuFolder,
} from 'react-icons/lu';
import { renderMarkdown, parseInternalHref } from '../markdown';
import { LuChevronDown, LuCheck, LuClapperboard, LuSparkles, LuPenTool, LuCuboid, LuDraftingCompass, LuPuzzle, LuType } from 'react-icons/lu';
import { cx } from '../format';

// ---------------------------------------------------------------- toasts
type ToastKind = 'info' | 'error' | 'success';
interface ToastAction { label: string; onClick: () => void }
interface ToastT { id: number; kind: ToastKind; text: string; action?: ToastAction }
const ToastCtx = createContext<(text: string, kind?: ToastKind, action?: ToastAction) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastT[]>([]);
  const push = useCallback((text: string, kind: ToastKind = 'info', action?: ToastAction) => {
    const id = Date.now() + Math.random();
    setItems((x) => [...x.slice(-2), { id, kind, text, action }]);
    // toasts with an action (e.g. Undo) stay long enough to use it
    setTimeout(() => setItems((x) => x.filter((t) => t.id !== id)), action ? 15000 : kind === 'error' ? 9000 : kind === 'success' ? 2500 : 4000);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toasts" role="status">
        {items.map((t) => (
          <div key={t.id} className={cx('toast', t.kind)}>
            {t.kind === 'error' ? <LuCircleAlert color="var(--bad)" /> : t.kind === 'success' ? <LuCircleCheck color="var(--ok)" /> : <LuInfo color="var(--info)" />}
            <div className="grow" style={{ whiteSpace: 'pre-wrap' }}>{t.text}</div>
            {t.action && <button className="btn sm" onClick={() => { setItems((x) => x.filter((y) => y.id !== t.id)); t.action!.onClick(); }}>{t.action.label}</button>}
            <button className="btn ghost sm icon" onClick={() => setItems((x) => x.filter((y) => y.id !== t.id))}><LuX /></button>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

/** Wraps an async action: shows errors as toasts, returns undefined on failure. */
export function useAction() {
  const toast = useToast();
  return useCallback(async <T,>(fn: () => Promise<T>, ok?: string): Promise<T | undefined> => {
    try {
      const r = await fn();
      if (ok) toast(ok, 'success');
      return r;
    } catch (e: any) {
      toast(e?.message || String(e), 'error');
      return undefined;
    }
  }, [toast]);
}

/** Loads data, re-running when deps change. */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[]): { data: T | undefined; error?: string; loading: boolean; reload: () => void } {
  const [state, setState] = useState<{ data?: T; error?: string; loading: boolean }>({ loading: true });
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let alive = true;
    setState((s) => ({ ...s, loading: true }));
    fn().then(
      (data) => alive && setState({ data, loading: false }),
      (e) => alive && setState({ error: e?.message || String(e), loading: false }),
    );
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);
  return { data: state.data, error: state.error, loading: state.loading, reload: useCallback(() => setTick((t) => t + 1), []) };
}

// ---------------------------------------------------------------- modal
export function Modal({ title, onClose, children, footer, wide }: { title: ReactNode; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [onClose]);
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={cx('modal', wide && 'wide')} role="dialog" aria-modal="true">
        <div className="mhead"><h2 className="grow">{title}</h2><button className="btn ghost icon" onClick={onClose} aria-label="Close"><LuX /></button></div>
        <div className="mbody">{children}</div>
        {footer && <div className="mfoot">{footer}</div>}
      </div>
    </div>
  );
}

/** Promise-based confirmation dialog. An optional checkbox reports its
 *  final state through `checkbox.onChange` before the promise resolves. */
type ConfirmOpts = {
  title: string; body: ReactNode; confirm?: string; danger?: boolean;
  checkbox?: { label: ReactNode; hint?: ReactNode; confirmChecked?: string; onChange: (on: boolean) => void };
};
const ConfirmCtx = createContext<(o: ConfirmOpts) => Promise<boolean>>(async () => false);
export const useConfirm = () => useContext(ConfirmCtx);
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<null | (ConfirmOpts & { resolve: (b: boolean) => void })>(null);
  const [checked, setChecked] = useState(false);
  const ask = useCallback((o: ConfirmOpts) => new Promise<boolean>((resolve) => { setChecked(false); setReq({ ...o, resolve }); }), []);
  const done = (b: boolean) => { req?.checkbox?.onChange(b && checked); req?.resolve(b); setReq(null); };
  const label = (checked && req?.checkbox?.confirmChecked) || req?.confirm || 'Confirm';
  return (
    <ConfirmCtx.Provider value={ask}>
      {children}
      {req && (
        <Modal title={req.title} onClose={() => done(false)} footer={<>
          <button className="btn" onClick={() => done(false)}>Cancel</button>
          <button className={cx('btn', req.danger ? 'danger' : 'primary')} autoFocus onClick={() => done(true)}>{label}</button>
        </>}>
          <div className="muted">{req.body}</div>
          {req.checkbox && (
            <label className={cx('confirm-check', checked && 'on')}>
              <input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
              <span><span className="cc-label">{req.checkbox.label}</span>{req.checkbox.hint && <span className="cc-hint">{req.checkbox.hint}</span>}</span>
            </label>
          )}
        </Modal>
      )}
    </ConfirmCtx.Provider>
  );
}

/** Promise-based text prompt (e.g. rename). `validate` returns an error message or null. */
type PromptOpts = { title: string; label?: string; value?: string; confirm?: string; selectUntilDot?: boolean; validate?: (v: string) => string | null; suggestions?: string[]; hint?: string };
const PromptCtx = createContext<(o: PromptOpts) => Promise<string | null>>(async () => null);
export const usePrompt = () => useContext(PromptCtx);
export function PromptProvider({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<null | (PromptOpts & { resolve: (v: string | null) => void })>(null);
  const [val, setVal] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const ask = useCallback((o: PromptOpts) => new Promise<string | null>((resolve) => { setVal(o.value || ''); setReq({ ...o, resolve }); }), []);
  useEffect(() => {
    if (!req || !input.current) return;
    const el = input.current;
    el.focus();
    const dot = req.selectUntilDot ? el.value.lastIndexOf('.') : -1;
    el.setSelectionRange(0, dot > 0 ? dot : el.value.length);
  }, [req]);
  const done = (v: string | null) => { req?.resolve(v); setReq(null); };
  const err = req?.validate?.(val) ?? null;
  return (
    <PromptCtx.Provider value={ask}>
      {children}
      {req && (
        <Modal title={req.title} onClose={() => done(null)} footer={<>
          <button className="btn" onClick={() => done(null)}>Cancel</button>
          <button className="btn primary" disabled={!!err} onClick={() => done(val)}>{req.confirm || 'OK'}</button>
        </>}>
          <div className="field" style={{ margin: 0 }}>
            {req.label && <label>{req.label}</label>}
            <input ref={input} className="input" value={val} list={req.suggestions?.length ? 'pv-prompt-sugg' : undefined} onChange={(e) => setVal(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && !err) done(val); }} />
            {req.suggestions?.length ? <datalist id="pv-prompt-sugg">{req.suggestions.map((x) => <option key={x} value={x} />)}</datalist> : null}
            {req.suggestions?.length ? <div className="row wrap" style={{ gap: 4, marginTop: 6 }}>{req.suggestions.map((x) => <button key={x} className={cx('chip', val === x && 'on')} onClick={() => setVal(x)}>{x}</button>)}</div> : null}
            {req.hint && <span className="small dim">{req.hint}</span>}
            {err && val !== (req.value || '') && <span className="small" style={{ color: 'var(--bad)' }}>{err}</span>}
          </div>
        </Modal>
      )}
    </PromptCtx.Provider>
  );
}

// ---------------------------------------------------------------- context menu
export interface MenuItem { label: string; icon?: ReactNode; onClick?: () => void; danger?: boolean; sep?: boolean; disabled?: boolean }
const MenuCtx = createContext<(e: { clientX: number; clientY: number; preventDefault(): void }, items: MenuItem[]) => void>(() => {});
export const useContextMenu = () => useContext(MenuCtx);
const MenuOpenCtx = createContext<() => boolean>(() => false);
/** Returns a function telling whether a context menu is currently open. */
export const useContextMenuOpen = () => useContext(MenuOpenCtx);
export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const [menu, setMenu] = useState<null | { x: number; y: number; items: MenuItem[] }>(null);
  const openRef = useRef(false);
  openRef.current = !!menu;
  const isOpen = useCallback(() => openRef.current, []);
  const menuEl = useRef<HTMLDivElement>(null);
  const returnFocus = useRef<Element | null>(null);
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const buttons = () => Array.from(menuEl.current?.querySelectorAll('button:not(:disabled)') || []) as HTMLButtonElement[];
    // keyboard: arrows/Home/End move, Enter/Space activate, Escape/Tab close
    const key = (e: KeyboardEvent) => {
      const bs = buttons();
      const i = bs.indexOf(document.activeElement as HTMLButtonElement);
      if (e.key === 'ArrowDown') { e.preventDefault(); e.stopPropagation(); bs[(i + 1) % bs.length]?.focus(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); bs[(i - 1 + bs.length) % bs.length]?.focus(); }
      else if (e.key === 'Home') { e.preventDefault(); bs[0]?.focus(); }
      else if (e.key === 'End') { e.preventDefault(); bs[bs.length - 1]?.focus(); }
      else if (e.key === 'Escape' || e.key === 'Tab') { e.preventDefault(); e.stopPropagation(); close(); (returnFocus.current as HTMLElement | null)?.focus?.(); }
    };
    returnFocus.current = document.activeElement;
    requestAnimationFrame(() => buttons()[0]?.focus());
    window.addEventListener('mousedown', close);
    window.addEventListener('blur', close);
    window.addEventListener('keydown', key, true);
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('blur', close); window.removeEventListener('keydown', key, true); };
  }, [menu]);
  const open = useCallback((e: any, items: MenuItem[]) => {
    e.preventDefault();
    // keep the whole menu on screen near edges
    const h = items.reduce((n, it) => n + (it.sep ? 9 : 31), 8);
    const x = Math.max(4, Math.min(e.clientX, window.innerWidth - 236));
    const y = e.clientY + h > window.innerHeight - 4 ? Math.max(4, e.clientY - h) : e.clientY;
    openRef.current = true;
    setMenu({ x, y, items });
  }, []);
  return (
    <MenuCtx.Provider value={open}><MenuOpenCtx.Provider value={isOpen}>
      {children}
      {menu && (
        <div className="ctx" role="menu" ref={menuEl} style={{ left: menu.x, top: menu.y }} onMouseDown={(e) => e.stopPropagation()}>
          {menu.items.map((it, i) => it.sep ? <hr key={i} /> : (
            <button key={i} role="menuitem" disabled={it.disabled} style={it.danger ? { color: 'var(--bad)' } : undefined} onClick={() => { setMenu(null); it.onClick?.(); }}>
              {it.icon}{it.label}
            </button>
          ))}
        </div>
      )}
    </MenuOpenCtx.Provider></MenuCtx.Provider>
  );
}

// ---------------------------------------------------------------- bits
export function Empty({ icon, title, children }: { icon?: ReactNode; title: string; children?: ReactNode }) {
  return <div className="empty">{icon}<h2>{title}</h2>{children}</div>;
}

export function TagInput({ value, onChange, placeholder }: { value: string[]; onChange: (v: string[]) => void; placeholder?: string }) {
  const [text, setText] = useState('');
  const add = () => {
    const t = text.trim().replace(/,$/, '');
    if (t && !value.some((v) => v.toLowerCase() === t.toLowerCase())) onChange([...value, t]);
    setText('');
  };
  return (
    <div className="input row wrap" style={{ gap: 4, padding: 4, minHeight: 32 }}>
      {value.map((t) => <span className="tag" key={t}>{t}<button aria-label={`Remove ${t}`} onClick={() => onChange(value.filter((x) => x !== t))}><LuX size={11} /></button></span>)}
      <input value={text} placeholder={value.length ? '' : placeholder || 'Add tag…'} onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); add(); } else if (e.key === 'Backspace' && !text && value.length) onChange(value.slice(0, -1)); }}
        onBlur={add} style={{ border: 0, background: 'transparent', outline: 'none', flex: 1, minWidth: 80, padding: '2px 4px' }} />
    </div>
  );
}

// Single-accent status dots: active work is lit, finished work is half-lit, the rest is gray.
export function statusDot(status: string): 'on' | 'half' | 'off' {
  const s = status.toLowerCase();
  if (s === 'active' || s === 'in progress') return 'on';
  if (s === 'completed' || s === 'done') return 'half';
  return 'off';
}
export function StatusPill({ status }: { status: string; colors?: Record<string, string> }) {
  return <span className="pill"><span className={`dot ${statusDot(status)}`} />{status}</span>;
}

const KIND_ICONS: Record<string, ReactNode> = {
  code: <LuFileCode />, document: <LuFileText />, config: <LuSettings2 />, image: <LuImage />, mesh: <LuBox />, model: <LuShapes />,
  gcode: <LuPrinter />, archive: <LuArchive />, engine: <LuGamepad2 />, audio: <LuMusic />, video: <LuVideo />, electronics: <LuCpu />, binary: <LuBinary />, font: <LuType />,
};
export function KindIcon({ kind }: { kind: string }) {
  return <span className="kind" title={kind}>{KIND_ICONS[kind] || <LuFile />}</span>;
}

const CAT: Record<string, { icon: ReactNode }> = {
  software: { icon: <LuCode /> },
  game: { icon: <LuGamepad2 /> },
  '3d-printing': { icon: <LuPrinter /> },
  electronics: { icon: <LuCpu /> },
  creative: { icon: <LuPalette /> },
  documents: { icon: <LuFileText /> },
  general: { icon: <LuFolder /> },
  addon: { icon: <LuPuzzle /> },
  video: { icon: <LuClapperboard /> },
  motion: { icon: <LuSparkles /> },
  design: { icon: <LuPenTool /> },
  '3d-art': { icon: <LuCuboid /> },
  cad: { icon: <LuDraftingCompass /> },
};
export function ProjectIcon({ category, icon, color, size = 38 }: { category: string; icon?: string; color?: string; size?: number }) {
  const c = CAT[category] || CAT.general;
  return (
    <div className={'picon ' + ({ software: 'c1', game: 'c3', addon: 'c3', '3d-printing': 'c1', '3d-art': 'c1', cad: 'c2', video: 'c3', motion: 'c3', design: 'c1', electronics: 'c2', creative: 'c3', documents: 'c2' }[category] || 'c2')} style={{ width: size, height: size, fontSize: size * 0.47, borderRadius: Math.max(5, size * 0.24) }}>
      {icon ? <span style={{ fontSize: size * 0.5 }}>{icon}</span> : c.icon}
    </div>
  );
}
export const GitIcon = LuFolderGit2;
export const WrenchIcon = LuWrench;

/** Rendered Markdown with clickable internal links. */
export function Markdown({ text, onInternalLink, className }: { text: string; onInternalLink?: (kind: 'note' | 'file' | 'task', target: string) => void; className?: string }) {
  const html = useMemo(() => renderMarkdown(text || ''), [text]);
  return (
    <div className={cx('md', className)} dangerouslySetInnerHTML={{ __html: html }}
      onClick={(e) => {
        const a = (e.target as HTMLElement).closest('a');
        if (!a) return;
        const href = a.getAttribute('href') || '';
        const il = parseInternalHref(href);
        e.preventDefault();
        if (il) onInternalLink?.(il.kind, il.target);
        else if (/^https?:/i.test(href)) import('../ipc').then((m) => m.isTauri ? import('@tauri-apps/plugin-opener').then((o: any) => o.openUrl?.(href)) : window.open(href, '_blank', 'noopener'));
      }} />
  );
}

/** Horizontal resizable split; width persisted per key. */
export function useResizable(key: string, initial: number, min = 180, max = 900) {
  const [w, setW] = useState<number>(() => {
    try { const v = Number(localStorage.getItem('pv.w.' + key)); return v >= min && v <= max ? v : initial; } catch { return initial; }
  });
  const [drag, setDrag] = useState(false);
  const start = useRef<{ x: number; w: number } | null>(null);
  useEffect(() => {
    if (!drag) return;
    const move = (e: MouseEvent) => { if (start.current) setW(Math.max(min, Math.min(max, start.current.w + e.clientX - start.current.x))); };
    const up = () => setDrag(false);
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
    document.body.style.cursor = 'col-resize';
    return () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); document.body.style.cursor = ''; };
  }, [drag, min, max]);
  useEffect(() => { try { localStorage.setItem('pv.w.' + key, String(w)); } catch { /* storage unavailable */ } }, [key, w]);
  const handle = <div className={cx('resizer', drag && 'drag')} onMouseDown={(e) => { start.current = { x: e.clientX, w }; setDrag(true); e.preventDefault(); }} />;
  return { width: w, handle };
}

/** Debounced autosave helper: returns [schedule, flush, state]. */
export function useAutosave(save: () => Promise<void>, delay = 800) {
  const t = useRef<number | undefined>(undefined);
  const saveRef = useRef(save);
  saveRef.current = save;
  const [state, setState] = useState<'idle' | 'pending' | 'saving' | 'saved' | 'error'>('idle');
  const flush = useCallback(async () => {
    if (t.current === undefined) return;
    clearTimeout(t.current);
    t.current = undefined;
    setState('saving');
    try { await saveRef.current(); setState('saved'); } catch { setState('error'); }
  }, []);
  const schedule = useCallback(() => {
    setState('pending');
    clearTimeout(t.current);
    t.current = window.setTimeout(() => { t.current = 0; flush(); }, delay);
  }, [delay, flush]);
  useEffect(() => () => { if (t.current !== undefined) { clearTimeout(t.current); saveRef.current().catch(() => {}); } }, []);
  return { schedule, flush, state };
}

export function SaveState({ state }: { state: string }) {
  const label = { idle: '', pending: 'Unsaved changes…', saving: 'Saving…', saved: 'Saved', error: 'Save failed — retrying on next edit' }[state] || '';
  return <span className="savestate" style={state === 'error' ? { color: 'var(--bad)' } : undefined}>{label}</span>;
}

// ---------------------------------------------------------------- dropdown menu
export interface DropdownSection { title?: string; items: { key: string; label: ReactNode; checked?: boolean; onSelect: () => void; hint?: string }[] }

/** Button that opens a menu of checkable items grouped in sections. */
export function Dropdown({ label, icon, sections, badge, align = 'left', width = 240 }: { label: ReactNode; icon?: ReactNode; sections: DropdownSection[]; badge?: number; align?: 'left' | 'right'; width?: number }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    window.addEventListener('mousedown', close);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('keydown', esc); };
  }, [open]);
  return (
    <div className="dropdown" ref={ref}>
      <button className={cx('btn', open && 'open')} onClick={() => setOpen(!open)} aria-haspopup="menu" aria-expanded={open}>
        {icon}{label}{badge ? <span className="badge ok">{badge}</span> : null}<LuChevronDown size={14} className="dim" />
      </button>
      {open && (
        <div className="dd-menu" role="menu" style={{ width, [align]: 0 }}>
          {sections.filter((s) => s.items.length).map((s, i) => (
            <div key={i} className="dd-section">
              {s.title && <div className="dd-title">{s.title}</div>}
              {s.items.map((it) => (
                <button key={it.key} role="menuitemcheckbox" aria-checked={!!it.checked} className={cx('dd-item', it.checked && 'on')} onClick={() => { it.onSelect(); }} title={it.hint}>
                  <span className="dd-check">{it.checked && <LuCheck size={13} />}</span><span className="grow">{it.label}</span>
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- tooltips
/**
 * App-wide tooltips: any element with a `title` (or `data-tip`) gets a styled
 * tooltip after a short hover, or immediately on keyboard focus. The title is
 * moved to data-tip on first use so the browser's own tooltip doesn't double up;
 * an aria-label is added when the element has no other accessible name.
 */
export function TooltipLayer() {
  const [tip, setTip] = useState<{ text: string; x: number; y: number; below: boolean } | null>(null);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let current: HTMLElement | null = null;
    const adopt = (el: HTMLElement) => {
      const t = el.getAttribute('title');
      if (t) {
        el.setAttribute('data-tip', t);
        el.removeAttribute('title');
        if (!el.getAttribute('aria-label') && !(el.textContent || '').trim()) el.setAttribute('aria-label', t);
      }
      return el.getAttribute('data-tip') || '';
    };
    const find = (n: EventTarget | null) => (n instanceof Element ? (n.closest('[title],[data-tip]') as HTMLElement | null) : null);
    const show = (el: HTMLElement) => {
      const text = adopt(el);
      if (!text || !el.isConnected) return;
      const r = el.getBoundingClientRect();
      const below = r.top < 60 || r.bottom + 40 < window.innerHeight;
      setTip({ text, x: Math.min(window.innerWidth - 12, Math.max(12, r.left + r.width / 2)), y: below ? r.bottom + 6 : r.top - 6, below });
    };
    const hide = () => { if (timer) clearTimeout(timer); timer = null; current = null; setTip(null); };
    const over = (e: PointerEvent) => {
      const el = find(e.target);
      if (el === current) return;
      hide();
      if (!el) return;
      current = el;
      adopt(el);
      timer = setTimeout(() => show(el), 450);
    };
    const focus = (e: FocusEvent) => {
      const el = find(e.target);
      if (!el || !(e.target as Element).matches?.(':focus-visible')) return;
      hide(); current = el; show(el);
    };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') hide(); };
    window.addEventListener('pointerover', over);
    window.addEventListener('pointerdown', hide, true);
    window.addEventListener('scroll', hide, true);
    window.addEventListener('focusin', focus);
    window.addEventListener('focusout', hide);
    window.addEventListener('keydown', key);
    window.addEventListener('blur', hide);
    return () => {
      hide();
      window.removeEventListener('pointerover', over);
      window.removeEventListener('pointerdown', hide, true);
      window.removeEventListener('scroll', hide, true);
      window.removeEventListener('focusin', focus);
      window.removeEventListener('focusout', hide);
      window.removeEventListener('keydown', key);
      window.removeEventListener('blur', hide);
    };
  }, []);
  if (!tip) return null;
  return <div className={cx('pv-tip', tip.below ? 'below' : 'above')} role="tooltip" style={{ left: tip.x, top: tip.y }}>{tip.text}</div>;
}
