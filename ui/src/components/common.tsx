import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  LuFileCode, LuFileText, LuSettings2, LuImage, LuBox, LuShapes, LuPrinter, LuArchive, LuGamepad2, LuMusic, LuVideo,
  LuCpu, LuBinary, LuFile, LuX, LuCircleAlert, LuCircleCheck, LuInfo, LuFolderGit2, LuCode, LuWrench, LuPalette, LuFolder,
} from 'react-icons/lu';
import { renderMarkdown, parseInternalHref } from '../markdown';
import { LuChevronDown, LuCheck } from 'react-icons/lu';
import { cx } from '../format';

// ---------------------------------------------------------------- toasts
type ToastKind = 'info' | 'error' | 'success';
interface ToastT { id: number; kind: ToastKind; text: string }
const ToastCtx = createContext<(text: string, kind?: ToastKind) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastT[]>([]);
  const push = useCallback((text: string, kind: ToastKind = 'info') => {
    const id = Date.now() + Math.random();
    setItems((x) => [...x.slice(-2), { id, kind, text }]);
    setTimeout(() => setItems((x) => x.filter((t) => t.id !== id)), kind === 'error' ? 9000 : kind === 'success' ? 2500 : 4000);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toasts" role="status">
        {items.map((t) => (
          <div key={t.id} className={cx('toast', t.kind)}>
            {t.kind === 'error' ? <LuCircleAlert color="var(--bad)" /> : t.kind === 'success' ? <LuCircleCheck color="var(--ok)" /> : <LuInfo color="var(--info)" />}
            <div className="grow" style={{ whiteSpace: 'pre-wrap' }}>{t.text}</div>
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

/** Promise-based confirmation dialog. */
const ConfirmCtx = createContext<(o: { title: string; body: ReactNode; confirm?: string; danger?: boolean }) => Promise<boolean>>(async () => false);
export const useConfirm = () => useContext(ConfirmCtx);
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<null | { title: string; body: ReactNode; confirm?: string; danger?: boolean; resolve: (b: boolean) => void }>(null);
  const ask = useCallback((o: any) => new Promise<boolean>((resolve) => setReq({ ...o, resolve })), []);
  const done = (b: boolean) => { req?.resolve(b); setReq(null); };
  return (
    <ConfirmCtx.Provider value={ask}>
      {children}
      {req && (
        <Modal title={req.title} onClose={() => done(false)} footer={<>
          <button className="btn" onClick={() => done(false)}>Cancel</button>
          <button className={cx('btn', req.danger ? 'danger' : 'primary')} autoFocus onClick={() => done(true)}>{req.confirm || 'Confirm'}</button>
        </>}>
          <div className="muted">{req.body}</div>
        </Modal>
      )}
    </ConfirmCtx.Provider>
  );
}

/** Promise-based text prompt (e.g. rename). `validate` returns an error message or null. */
type PromptOpts = { title: string; label?: string; value?: string; confirm?: string; selectUntilDot?: boolean; validate?: (v: string) => string | null };
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
            <input ref={input} className="input" value={val} onChange={(e) => setVal(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && !err) done(val); }} />
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
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener('mousedown', close);
    window.addEventListener('blur', close);
    window.addEventListener('keydown', close);
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('blur', close); window.removeEventListener('keydown', close); };
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
        <div className="ctx" style={{ left: menu.x, top: menu.y }} onMouseDown={(e) => e.stopPropagation()}>
          {menu.items.map((it, i) => it.sep ? <hr key={i} /> : (
            <button key={i} disabled={it.disabled} style={it.danger ? { color: 'var(--bad)' } : undefined} onClick={() => { setMenu(null); it.onClick?.(); }}>
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
  gcode: <LuPrinter />, archive: <LuArchive />, engine: <LuGamepad2 />, audio: <LuMusic />, video: <LuVideo />, electronics: <LuCpu />, binary: <LuBinary />,
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
};
export function ProjectIcon({ category, icon, color, size = 38 }: { category: string; icon?: string; color?: string; size?: number }) {
  const c = CAT[category] || CAT.general;
  return (
    <div className={'picon ' + ({ software: 'c1', game: 'c3', '3d-printing': 'c1', electronics: 'c2', creative: 'c3', documents: 'c2' }[category] || 'c2')} style={{ width: size, height: size, fontSize: size * 0.47, borderRadius: Math.max(5, size * 0.24) }}>
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
