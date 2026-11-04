// Single bridge to the Rust core. Inside Tauri it uses the `vault_call`
// command; in a plain browser (dev server / automated UI tests) it POSTs to
// /api/<cmd>. Both reach the same `vault_core::api::call`.
import { invoke } from '@tauri-apps/api/core';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export class VaultError extends Error {}

export async function call<T = any>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  let raw: string;
  if (isTauri) {
    raw = await invoke<string>('vault_call', { cmd, args: JSON.stringify(args) });
  } else {
    const r = await fetch('/api/' + cmd, { method: 'POST', body: JSON.stringify(args) });
    raw = await r.text();
  }
  const env = JSON.parse(raw);
  if (!env.ok) throw new VaultError(env.error);
  return env.data as T;
}

// ---- native helpers (no-ops with a clear message outside the desktop app)

export async function pickFolder(title: string): Promise<string | null> {
  if (!isTauri) return window.prompt(title + ' (enter a folder path)') || null;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ directory: true, multiple: false, title });
  return typeof r === 'string' ? r : null;
}

export async function pickFiles(title: string, filters?: { name: string; extensions: string[] }[]): Promise<string[]> {
  if (!isTauri) {
    const s = window.prompt(title + ' (enter file paths separated by ;)');
    return s ? s.split(';').map((x) => x.trim()).filter(Boolean) : [];
  }
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ multiple: true, title, filters });
  return Array.isArray(r) ? r : typeof r === 'string' ? [r] : [];
}

export async function openPath(path: string) {
  if (!isTauri) throw new VaultError('Opening files with their default application is available in the desktop app.');
  const { openPath } = await import('@tauri-apps/plugin-opener');
  await openPath(path);
}

export async function revealPath(path: string) {
  if (!isTauri) throw new VaultError('Showing files in Explorer is available in the desktop app.');
  const { revealItemInDir } = await import('@tauri-apps/plugin-opener');
  await revealItemInDir(path);
}

export async function onFileDrop(cb: (paths: string[]) => void, onHover?: (over: boolean) => void): Promise<() => void> {
  if (!isTauri) return () => {};
  const { getCurrentWebview } = await import('@tauri-apps/api/webview');
  return getCurrentWebview().onDragDropEvent((e: any) => {
    const t = e.payload.type;
    if (t === 'over' || t === 'enter') onHover?.(true);
    else if (t === 'leave') onHover?.(false);
    else if (t === 'drop') {
      onHover?.(false);
      cb(e.payload.paths || []);
    }
  });
}

// ---- types mirrored from vault-core JSON

export interface Project {
  id: string; name: string; description: string; category: string; status: string;
  mode: 'linked' | 'managed'; rootPath: string; rootAvailable: boolean; icon: string; color: string;
  progress: number; favorite: boolean; group: string; module: Record<string, any>; ignoreGlobs: string;
  createdAt: number; updatedAt: number; lastOpenedAt?: number; lastScanAt?: number; tags: string[];
  fileCount: number; bytes: number; missingFiles: number; changedFiles: number; pendingReviews: number; staleDocs?: number;
  openTasks: number; latestActivity?: string; latestActivityAt?: number; lastFile?: string; lastFileAt?: number; recentFiles?: { path: string; at: number }[];
}
export interface VFile {
  id: string; path: string; name: string; ext: string; kind: string; size: number; mtime: number;
  sha256?: string; state: 'present' | 'missing' | 'deleted'; firstSeen: number; lastChanged: number;
  changedSinceReview: boolean; hasDocs: boolean; purpose: string; versions: number;
}
export interface LinkRef { linkId: string; type: 'file' | 'note' | 'task'; id: string; title: string; state: string; label: string }
export interface FileDocs { purpose: string; notes: string; warnings: string; custom: Record<string, string>; updatedAt?: number; fileChangedSinceDoc: boolean }
export interface FileDetail extends VFile {
  projectId: string; absPath: string; tags: string[]; docs: FileDocs;
  events: { id: string; kind: string; detail: string; at: number; /** your "why" */ note?: string; sha256?: string; /** content changes: before and after, for a diff */ from?: string; to?: string }[]; links: LinkRef[];
  duplicates: { id: string; path: string }[]; meshDimensionsMm?: number[];
}
export interface Note { id: string; projectId: string; parentId?: string; title: string; category: string; sort: number; createdAt: number; updatedAt: number; body?: string; links?: LinkRef[] }
export interface Task {
  id: string; projectId: string; kind: 'task' | 'bug'; title: string; description: string; status: string; priority: number;
  dueAt?: number; milestoneId?: string; checklist: { text: string; done: boolean }[]; reproSteps: string; expected: string;
  actual: string; resolution: string; createdAt: number; updatedAt: number; completedAt?: number; tags: string[]; links?: LinkRef[];
}
export interface ScanReport { versioned: number; total: number; added: number; modified: number; moved: number; missing: number; restored: number; needsReview: number; errors: string[]; errorCount: number }
