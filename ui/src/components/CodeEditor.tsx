import { useEffect, useRef } from 'react';
import type { EditorHandle } from '../../editor/api';
import { getTheme } from '../theme';

import type { DiffHandle } from '../../editor/api';
type EditorModule = { create: (el: HTMLElement, o: any) => EditorHandle; createDiff?: (el: HTMLElement, o: any) => DiffHandle };
let loader: Promise<EditorModule> | null = null;

/** Loads the separately bundled editor (Monaco) once, plus its stylesheet. */
export function loadEditor(): Promise<EditorModule> {
  if (!loader) {
    const css = document.createElement('link');
    css.rel = 'stylesheet';
    css.href = new URL('./monaco.css', import.meta.url).href;
    document.head.appendChild(css);
    const url = new URL('./monaco.js', import.meta.url).href;
    loader = import(/* runtime-loaded bundle */ url);
  }
  return loader;
}

const LANG: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript', json: 'json',
  rs: 'rust', py: 'python', cs: 'csharp', c: 'c', h: 'cpp', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', ino: 'cpp', gsc: 'gsc', csc: 'gsc', gsh: 'gsc',
  go: 'go', java: 'java', kt: 'kotlin', lua: 'lua', rb: 'ruby', php: 'php', swift: 'swift', sql: 'sql', sh: 'shell', bat: 'bat',
  cmd: 'bat', ps1: 'powershell', css: 'css', scss: 'scss', less: 'less', html: 'html', htm: 'html', xml: 'xml', svg: 'xml',
  yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini', cfg: 'ini', conf: 'ini', md: 'markdown', markdown: 'markdown',
  shader: 'hlsl', hlsl: 'hlsl', glsl: 'cpp', gd: 'python', dockerfile: 'dockerfile', graphql: 'graphql',
};
export const languageFor = (ext: string) => LANG[ext.toLowerCase()] || 'plaintext';

export function CodeEditor({ value, ext, readOnly, onChange, onSave, onSaveWithBackup, onReady, onGoToDefinition, onHover }: {
  value: string; ext: string; readOnly?: boolean; onChange: (v: string) => void; onSave: () => void; onSaveWithBackup?: () => void; onReady?: (h: EditorHandle) => void;
  onGoToDefinition?: (word: string, qualifier: string) => void; onHover?: (word: string, qualifier: string) => Promise<string | null>;
}) {
  const el = useRef<HTMLDivElement>(null);
  const handle = useRef<EditorHandle | null>(null);
  const cb = useRef({ onChange, onSave, onSaveWithBackup, onGoToDefinition, onHover });
  cb.current = { onChange, onSave, onSaveWithBackup, onGoToDefinition, onHover };
  const hasDef = !!onGoToDefinition, hasHover = !!onHover;

  useEffect(() => {
    let disposed = false;
    loadEditor().then((m) => {
      if (disposed || !el.current) return;
      const t = getTheme();
      handle.current = m.create(el.current, {
        value, language: languageFor(ext), readOnly, palette: { a1: t.a1, a2: t.a2, a3: t.a3 },
        onChange: (v: string) => cb.current.onChange(v), onSave: () => cb.current.onSave(), onSaveWithBackup: () => cb.current.onSaveWithBackup?.(),
        onGoToDefinition: hasDef ? (w: string, q: string) => cb.current.onGoToDefinition?.(w, q) : undefined,
        onHover: hasHover ? (w: string, q: string) => cb.current.onHover?.(w, q) ?? Promise.resolve(null) : undefined,
      });
      handle.current.focus();
      onReady?.(handle.current);
    });
    const onTheme = (e: Event) => { const t = (e as CustomEvent).detail; handle.current?.setPalette({ a1: t.a1, a2: t.a2, a3: t.a3 }); };
    window.addEventListener('pv-theme', onTheme);
    return () => { disposed = true; window.removeEventListener('pv-theme', onTheme); handle.current?.dispose(); handle.current = null; };
    // the editor owns its content after creation; `value` is only the initial text
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={el} className="pv-editor" />;
}

/** Monaco diff editor when available; returns false so callers can fall back. */
export function MonacoDiff({ original, modified, ext, sideBySide, onUnavailable, handleRef, onChanges }: { original: string; modified: string; ext: string; sideBySide: boolean; onUnavailable: () => void; handleRef?: { current: DiffHandle | null }; onChanges?: (n: number) => void }) {
  const el = useRef<HTMLDivElement>(null);
  const h = useRef<DiffHandle | null>(null);
  useEffect(() => {
    let gone = false;
    loadEditor().then((m) => {
      if (gone || !el.current) return;
      if (!m.createDiff) { onUnavailable(); return; }
      const t = getTheme();
      h.current = m.createDiff(el.current, { original, modified, language: languageFor(ext), sideBySide, palette: { a1: t.a1, a2: t.a2, a3: t.a3 }, onChanges });
      if (handleRef) handleRef.current = h.current;
    }).catch(() => { if (!gone) onUnavailable(); });
    return () => { gone = true; h.current?.dispose(); h.current = null; if (handleRef) handleRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [original, modified, ext]);
  useEffect(() => { h.current?.setSideBySide(sideBySide); }, [sideBySide]);
  return <div ref={el} className="pv-diff" />;
}
