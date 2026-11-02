// Monaco-based editor, bundled as its own file (ui/dist/monaco.js + monaco.css)
// and loaded on first use. Workers are bundled next to it; nothing is fetched
// from the internet.
import * as monaco from 'monaco-editor';
import { wordAt, type DiffHandle, type DiffOptions, type EditorHandle, type EditorOptions } from './api';

// ---- GSC / CSC (Call of Duty scripts)
let gscReady = false;
function registerGsc() {
  if (gscReady) return;
  gscReady = true;
  monaco.languages.register({ id: 'gsc', extensions: ['.gsc', '.csc', '.gsh'], aliases: ['GSC', 'gsc'] });
  monaco.languages.setLanguageConfiguration('gsc', {
    comments: { lineComment: '//', blockComment: ['/*', '*/'] },
    brackets: [['{', '}'], ['[', ']'], ['(', ')']],
    autoClosingPairs: [{ open: '{', close: '}' }, { open: '[', close: ']' }, { open: '(', close: ')' }, { open: '"', close: '"', notIn: ['string'] }],
    surroundingPairs: [{ open: '{', close: '}' }, { open: '(', close: ')' }, { open: '"', close: '"' }],
    indentationRules: { increaseIndentPattern: /^.*\{[^}"']*$/, decreaseIndentPattern: /^\s*\}/ },
  });
  monaco.languages.setMonarchTokensProvider('gsc', {
    ignoreCase: true,
    keywords: ['if', 'else', 'for', 'foreach', 'in', 'while', 'do', 'switch', 'case', 'default', 'break', 'continue', 'return', 'function', 'private', 'autoexec', 'class', 'var', 'const'],
    threading: ['thread', 'childthread', 'wait', 'waittill', 'waittillmatch', 'waittillframeend', 'waitframe', 'notify', 'endon', 'isdefined'],
    builtins: ['self', 'level', 'game', 'anim', 'world', 'undefined', 'true', 'false'],
    tokenizer: {
      root: [
        [/^\s*#\s*(include|using|insert|define|if|ifdef|ifndef|else|endif|using_animtree)\b.*$/, 'annotation'],
        [/\/\/.*$/, 'comment'],
        [/\/\*/, 'comment', '@comment'],
        [/"([^"\\]|\\.)*$/, 'string.invalid'],
        [/"/, 'string', '@string'],
        [/&"/, 'string', '@string'],
        [/%[A-Za-z0-9_]+/, 'type'],
        [/#animtree/, 'type'],
        [/\d+(\.\d+)?/, 'number'],
        [/[A-Za-z_][\w\\]*::/, 'type'],
        [/[A-Za-z_]\w*(?=\s*\()/, { cases: { '@keywords': 'keyword', '@threading': 'keyword.control', '@default': 'variable' } }],
        [/[A-Za-z_]\w*/, { cases: { '@keywords': 'keyword', '@threading': 'keyword.control', '@builtins': 'storage', '@default': 'identifier' } }],
        [/[{}()[\]]/, 'delimiter'],
        [/[=><!~?:&|+\-*/^%]+/, 'operator'],
        [/[;,.]/, 'delimiter'],
      ],
      comment: [[/[^/*]+/, 'comment'], [/\*\//, 'comment', '@pop'], [/[/*]/, 'comment']],
      string: [[/[^\\"]+/, 'string'], [/\\./, 'string.escape'], [/"/, 'string', '@pop']],
    },
  } as any);
}

(self as any).MonacoEnvironment = {
  getWorkerUrl(_id: string, label: string) {
    if (label === 'json') return './json.worker.js';
    if (label === 'css' || label === 'scss' || label === 'less') return './css.worker.js';
    if (label === 'html' || label === 'handlebars' || label === 'razor') return './html.worker.js';
    if (label === 'typescript' || label === 'javascript') return './ts.worker.js';
    return './editor.worker.js';
  },
};

const strip = (c: string) => c.replace('#', '');

function defineTheme(p: { a1: string; a2: string; a3: string }) {
  // Syntax colors come only from the app's palette (+ grays), so the editor
  // matches whatever theme is chosen.
  monaco.editor.defineTheme('pv-black', {
    base: 'vs-dark',
    inherit: false,
    rules: [
      { token: '', foreground: 'd4d4d4' },
      { token: 'comment', foreground: '5c5c5c', fontStyle: 'italic' },
      { token: 'keyword', foreground: strip(p.a1) },
      { token: 'keyword.control', foreground: strip(p.a1) },
      { token: 'storage', foreground: strip(p.a1) },
      { token: 'string', foreground: strip(p.a2) },
      { token: 'string.escape', foreground: strip(p.a3) },
      { token: 'number', foreground: strip(p.a3) },
      { token: 'regexp', foreground: strip(p.a3) },
      { token: 'type', foreground: strip(p.a2), fontStyle: 'bold' },
      { token: 'type.identifier', foreground: 'e5e5e5' },
      { token: 'identifier', foreground: 'd4d4d4' },
      { token: 'delimiter', foreground: '8a8a8a' },
      { token: 'operator', foreground: '8a8a8a' },
      { token: 'tag', foreground: strip(p.a1) },
      { token: 'attribute.name', foreground: strip(p.a2) },
      { token: 'attribute.value', foreground: strip(p.a3) },
      { token: 'key', foreground: strip(p.a1) },
      { token: 'variable', foreground: 'e5e5e5' },
      { token: 'annotation', foreground: strip(p.a3) },
    ],
    colors: {
      'editor.background': '#000000',
      'editor.foreground': '#d4d4d4',
      'editorLineNumber.foreground': '#3a3a3a',
      'editorLineNumber.activeForeground': p.a1,
      'editor.lineHighlightBackground': '#0c0c0c',
      'editor.lineHighlightBorder': '#00000000',
      'editorCursor.foreground': p.a1,
      'editor.selectionBackground': p.a1 + '40',
      'editor.inactiveSelectionBackground': p.a1 + '22',
      'editor.findMatchBackground': p.a2 + '55',
      'editor.findMatchHighlightBackground': p.a2 + '30',
      'editorIndentGuide.background1': '#1a1a1a',
      'editorIndentGuide.activeBackground1': '#333333',
      'editorWhitespace.foreground': '#2a2a2a',
      'editorBracketMatch.background': p.a1 + '22',
      'editorBracketMatch.border': p.a1 + '88',
      'scrollbarSlider.background': '#ffffff14',
      'scrollbarSlider.hoverBackground': '#ffffff22',
      'editorWidget.background': '#101010',
      'editorWidget.border': '#2a2a2a',
      'input.background': '#000000',
      'focusBorder': p.a1 + '88',
      'minimap.background': '#000000',
      'diffEditor.insertedTextBackground': p.a1 + '33',
      'diffEditor.insertedLineBackground': p.a1 + '1a',
      'diffEditor.removedTextBackground': p.a3 + '33',
      'diffEditor.removedLineBackground': p.a3 + '1a',
      'diffEditor.diagonalFill': '#ffffff10',
    },
  });
}

export function create(el: HTMLElement, o: EditorOptions): EditorHandle {
  defineTheme(o.palette);
  registerGsc();
  const ed = monaco.editor.create(el, {
    value: o.value,
    language: o.language,
    readOnly: o.readOnly,
    theme: 'pv-black',
    automaticLayout: true,
    fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
    fontSize: 13,
    fontLigatures: true,
    minimap: { enabled: true, renderCharacters: false },
    scrollBeyondLastLine: false,
    renderWhitespace: 'selection',
    smoothScrolling: true,
    tabSize: 4,
    detectIndentation: true,
    padding: { top: 10 },
  });
  ed.onDidChangeModelContent(() => o.onChange(ed.getValue()));
  ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => o.onSave());
  ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyS, () => o.onSaveWithBackup?.());
  // Go to definition (F12 / Ctrl+click) is answered by the app, which knows the project's files.
  const wordAtPos = (pos: monaco.Position | null) => {
    const m = ed.getModel();
    if (!pos || !m) return null;
    return wordAt(m.getLineContent(pos.lineNumber), pos.column - 1);
  };
  if (o.onGoToDefinition) {
    ed.addAction({
      id: 'pv.goToDefinition', label: 'Go to Definition', keybindings: [monaco.KeyCode.F12], contextMenuGroupId: 'navigation', contextMenuOrder: 0,
      run: () => { const w = wordAtPos(ed.getPosition()); if (w) o.onGoToDefinition!(w.word, w.qualifier); },
    });
    ed.onMouseDown((e) => {
      if ((e.event.ctrlKey || e.event.metaKey) && e.target.position) {
        const w = wordAtPos(e.target.position);
        if (w) { e.event.preventDefault(); o.onGoToDefinition!(w.word, w.qualifier); }
      }
    });
  }
  let hoverReg: monaco.IDisposable | null = null;
  if (o.onHover) {
    hoverReg = monaco.languages.registerHoverProvider(o.language, {
      provideHover: async (model, pos) => {
        if (model !== ed.getModel()) return null;
        const w = wordAt(model.getLineContent(pos.lineNumber), pos.column - 1);
        if (!w) return null;
        const text = await o.onHover!(w.word, w.qualifier);
        return text ? { contents: [{ value: text }] } : null;
      },
    });
  }
  return {
    revealLine: (line) => { ed.revealLineInCenter(line); ed.setPosition({ lineNumber: line, column: 1 }); ed.focus(); },
    getValue: () => ed.getValue(),
    setValue: (v) => ed.setValue(v),
    focus: () => ed.focus(),
    setPalette: (p) => { defineTheme(p); monaco.editor.setTheme('pv-black'); },
    dispose: () => { hoverReg?.dispose(); ed.getModel()?.dispose(); ed.dispose(); },
  };
}

/** Read-only side-by-side (or inline) diff using Monaco's diff editor. */
export function createDiff(el: HTMLElement, o: DiffOptions): DiffHandle {
  defineTheme(o.palette);
  const original = monaco.editor.createModel(o.original, o.language);
  const modified = monaco.editor.createModel(o.modified, o.language);
  const d = monaco.editor.createDiffEditor(el, {
    theme: 'pv-black', readOnly: true, originalEditable: false, automaticLayout: true, renderSideBySide: o.sideBySide,
    fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace", fontSize: 12.5, minimap: { enabled: false }, scrollBeyondLastLine: false,
  });
  d.setModel({ original, modified });
  return {
    setSideBySide: (v) => d.updateOptions({ renderSideBySide: v }),
    dispose: () => { d.dispose(); original.dispose(); modified.dispose(); },
  };
}
