// Shared contract between the app and the separately bundled editor (monaco.js).
export interface EditorOptions {
  value: string;
  language: string;
  readOnly?: boolean;
  palette: { a1: string; a2: string; a3: string };
  onChange: (value: string) => void;
  onSave: () => void;
  /** Ctrl+Shift+S: open the Smart Save (backup) dialog */
  onSaveWithBackup?: () => void;
  /** F12 / Ctrl+click on a word: `qualifier` is the script path before `::`, if any */
  onGoToDefinition?: (word: string, qualifier: string) => void;
  /** Hover text for a word (e.g. a function signature and where it is defined) */
  onHover?: (word: string, qualifier: string) => Promise<string | null>;
}

/** Word under the cursor plus an optional `path::` qualifier in front of it. */
export function wordAt(line: string, col: number): { word: string; qualifier: string } | null {
  const isId = (c: string) => /[A-Za-z0-9_]/.test(c);
  let s = col, e = col;
  while (s > 0 && isId(line[s - 1])) s--;
  while (e < line.length && isId(line[e])) e++;
  const word = line.slice(s, e);
  if (!word || /^\d/.test(word)) return null;
  let qualifier = '';
  if (line.slice(Math.max(0, s - 2), s) === '::') {
    let q = s - 2;
    while (q > 0 && /[A-Za-z0-9_\\/]/.test(line[q - 1])) q--;
    qualifier = line.slice(q, s - 2);
  }
  return { word, qualifier };
}
export interface DiffOptions {
  original: string;
  modified: string;
  language: string;
  sideBySide: boolean;
  palette: { a1: string; a2: string; a3: string };
}
export interface DiffHandle { setSideBySide(v: boolean): void; dispose(): void }
export interface EditorHandle {
  /** Scrolls to a 1-based line and puts the cursor there */
  revealLine(line: number): void;
  getValue(): string;
  setValue(v: string): void;
  focus(): void;
  setPalette(p: { a1: string; a2: string; a3: string }): void;
  dispose(): void;
}
