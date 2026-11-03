/** Which open editors have unsaved changes (by file id, with their project),
 *  so closing a project tab can ask first. */
const dirty = new Map<string, { name: string; projectId?: string }>();

export function markDirty(fileId: string, name: string, isDirty: boolean, projectId?: string) {
  if (isDirty) dirty.set(fileId, { name, projectId }); else dirty.delete(fileId);
  window.dispatchEvent(new CustomEvent('pv:dirty'));
}
export function forgetEditor(fileId: string) { if (dirty.delete(fileId)) window.dispatchEvent(new CustomEvent('pv:dirty')); }
export function unsavedNames(projectId?: string): string[] {
  return [...dirty.values()].filter((d) => !projectId || d.projectId === projectId).map((d) => d.name);
}

function ask(names: string[]): boolean {
  if (!names.length) return true;
  return window.confirm(`Unsaved changes in ${names.length === 1 ? names[0] : `${names.length} files (${names.slice(0, 3).join(', ')}${names.length > 3 ? '…' : ''})`}. Discard them?`);
}

/** True when it's fine to close every editor (nothing unsaved, or the user agreed). */
export function confirmDiscardAll(): boolean {
  const ok = ask(unsavedNames());
  if (ok) { dirty.clear(); window.dispatchEvent(new CustomEvent('pv:dirty')); }
  return ok;
}

/** Same, for one project's editors (closing its tab). */
export function confirmDiscardProject(projectId: string): boolean {
  const ok = ask(unsavedNames(projectId));
  if (ok) {
    for (const [k, d] of dirty) if (d.projectId === projectId) dirty.delete(k);
    window.dispatchEvent(new CustomEvent('pv:dirty'));
  }
  return ok;
}
