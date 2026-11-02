export function bytes(n: number): string {
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
}

export function ago(ms?: number | null): string {
  if (!ms) return '—';
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)} d ago`;
  return new Date(ms).toLocaleDateString();
}

export function dateTime(ms?: number | null): string {
  return ms ? new Date(ms).toLocaleString() : '—';
}

export function day(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { weekday: 'short', year: 'numeric', month: 'short', day: 'numeric' });
}

export const CATEGORY_LABELS: Record<string, string> = {
  general: 'General', software: 'Software', game: 'Game development', addon: 'Game addon / mod',
  '3d-printing': '3D printing', '3d-art': '3D art & animation', cad: 'CAD & engineering',
  video: 'Video editing', motion: 'Motion graphics', design: '2D design & illustration',
  electronics: 'Electronics', creative: 'Creative (other)', documents: 'Documents',
};

/** Project types the user has hidden (Settings → Project types). Stored per computer. */
export function hiddenTypes(): string[] {
  try { return JSON.parse(localStorage.getItem('pv.hiddenTypes') || '[]'); } catch { return []; }
}
export function setHiddenTypes(t: string[]) {
  try { localStorage.setItem('pv.hiddenTypes', JSON.stringify(t)); } catch { /* ignore */ }
  window.dispatchEvent(new CustomEvent('pv:types'));
}
/** Types to offer in pickers: enabled ones, plus `keep` (a project's current type) so it never disappears. */
export function typeChoices(keep?: string): [string, string][] {
  const hidden = new Set(hiddenTypes());
  return Object.entries(CATEGORY_LABELS).filter(([k]) => !hidden.has(k) || k === keep);
}

export function cx(...c: (string | false | null | undefined)[]) {
  return c.filter(Boolean).join(' ');
}

export function slug(s: string) {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'project';
}
