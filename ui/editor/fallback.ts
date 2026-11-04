// Plain-text stand-in with the same API as monaco.ts. Only used when the
import './fallback.css';
// monaco-editor package isn't installed (offline test environments).
import { wordAt, type EditorHandle, type EditorOptions } from './api';

export function create(el: HTMLElement, o: EditorOptions): EditorHandle {
  const ta = document.createElement('textarea');
  ta.className = 'pv-fallback-editor';
  ta.value = o.value;
  ta.readOnly = !!o.readOnly;
  ta.spellcheck = false;
  ta.addEventListener('input', () => o.onChange(ta.value));
  ta.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); if (e.shiftKey) o.onSaveWithBackup?.(); else o.onSave(); }
    if (e.key === 'F12' && o.onGoToDefinition) {
      e.preventDefault();
      const pos = ta.selectionStart;
      const ls = ta.value.lastIndexOf('\n', pos - 1) + 1;
      const le = ta.value.indexOf('\n', pos);
      const w = wordAt(ta.value.slice(ls, le < 0 ? undefined : le), pos - ls);
      if (w) o.onGoToDefinition(w.word, w.qualifier);
    }
    if (e.key === 'Tab') { e.preventDefault(); const s = ta.selectionStart; ta.setRangeText('    ', s, ta.selectionEnd, 'end'); o.onChange(ta.value); }
  });
  el.appendChild(ta);
  return {
    revealLine: (line, sel) => {
      const lines = ta.value.split('\n');
      const at = (ln: number, col: number) => lines.slice(0, Math.max(0, ln - 1)).reduce((a, l) => a + l.length + 1, 0) + col - 1;
      const off = at(line, 1);
      ta.focus();
      if (sel) ta.setSelectionRange(at(sel.line, sel.col), at(sel.endLine, sel.endCol));
      else ta.setSelectionRange(off, off + (lines[line - 1]?.length || 0));
      const lh = parseFloat(getComputedStyle(ta).lineHeight) || 18;
      ta.scrollTop = Math.max(0, (line - 4) * lh);
    },
    getValue: () => ta.value,
    setValue: (v) => { ta.value = v; },
    focus: () => ta.focus(),
    setPalette: () => {},
    dispose: () => ta.remove(),
  };
}

// The stand-in has no diff view; the app falls back to its own diff renderer.
export const createDiff = undefined;
