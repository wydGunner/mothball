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
  getValue(): string;
  setValue(v: string): void;
  focus(): void;
  setPalette(p: { a1: string; a2: string; a3: string }): void;
  dispose(): void;
}
