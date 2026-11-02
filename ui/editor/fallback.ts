// Plain-text stand-in with the same API as monaco.ts. Only used when the
import './fallback.css';
// monaco-editor package isn't installed (offline test environments).
import type { EditorHandle, EditorOptions } from './api';

export function create(el: HTMLElement, o: EditorOptions): EditorHandle {
  const ta = document.createElement('textarea');
  ta.className = 'pv-fallback-editor';
  ta.value = o.value;
  ta.readOnly = !!o.readOnly;
  ta.spellcheck = false;
  ta.addEventListener('input', () => o.onChange(ta.value));
  ta.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); o.onSave(); }
    if (e.key === 'Tab') { e.preventDefault(); const s = ta.selectionStart; ta.setRangeText('    ', s, ta.selectionEnd, 'end'); o.onChange(ta.value); }
  });
  el.appendChild(ta);
  return {
    getValue: () => ta.value,
    setValue: (v) => { ta.value = v; },
    focus: () => ta.focus(),
    setPalette: () => {},
    dispose: () => ta.remove(),
  };
}
