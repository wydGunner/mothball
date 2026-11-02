import { useEffect, useRef } from 'react';
import type { EditorHandle } from '../../editor/api';
import { getTheme } from '../theme';

let loader: Promise<{ create: (el: HTMLElement, o: any) => EditorHandle }> | null = null;

/** Loads the separately bundled editor (Monaco) once, plus its stylesheet. */
function loadEditor() {
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
  rs: 'rust', py: 'python', cs: 'csharp', c: 'c', h: 'cpp', cpp: 'cpp', hpp: 'cpp', cc: 'cpp', ino: 'cpp', gsc: 'cpp', csc: 'cpp',
  go: 'go', java: 'java', kt: 'kotlin', lua: 'lua', rb: 'ruby', php: 'php', swift: 'swift', sql: 'sql', sh: 'shell', bat: 'bat',
  cmd: 'bat', ps1: 'powershell', css: 'css', scss: 'scss', less: 'less', html: 'html', htm: 'html', xml: 'xml', svg: 'xml',
  yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini', cfg: 'ini', conf: 'ini', md: 'markdown', markdown: 'markdown',
  shader: 'hlsl', hlsl: 'hlsl', glsl: 'cpp', gd: 'python', dockerfile: 'dockerfile', graphql: 'graphql',
};
export const languageFor = (ext: string) => LANG[ext.toLowerCase()] || 'plaintext';

export function CodeEditor({ value, ext, readOnly, onChange, onSave, onReady }: {
  value: string; ext: string; readOnly?: boolean; onChange: (v: string) => void; onSave: () => void; onReady?: (h: EditorHandle) => void;
}) {
  const el = useRef<HTMLDivElement>(null);
  const handle = useRef<EditorHandle | null>(null);
  const cb = useRef({ onChange, onSave });
  cb.current = { onChange, onSave };

  useEffect(() => {
    let disposed = false;
    loadEditor().then((m) => {
      if (disposed || !el.current) return;
      const t = getTheme();
      handle.current = m.create(el.current, {
        value, language: languageFor(ext), readOnly, palette: { a1: t.a1, a2: t.a2, a3: t.a3 },
        onChange: (v: string) => cb.current.onChange(v), onSave: () => cb.current.onSave(),
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
