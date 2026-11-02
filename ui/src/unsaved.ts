/** Which open editors have unsaved changes (by file id), so navigation that
 *  would close them can ask first. */
const dirty = new Map<string, string>();

export function markDirty(fileId: string, name: string, isDirty: boolean) {
  if (isDirty) dirty.set(fileId, name); else dirty.delete(fileId);
}
export function forgetEditor(fileId: string) { dirty.delete(fileId); }
export function unsavedNames(): string[] { return [...dirty.values()]; }

/** True when it's fine to close every editor (nothing unsaved, or the user agreed). */
export function confirmDiscardAll(): boolean {
  const names = unsavedNames();
  if (!names.length) return true;
  const ok = window.confirm(`Unsaved changes in ${names.length === 1 ? names[0] : `${names.length} files (${names.slice(0, 3).join(', ')}${names.length > 3 ? '…' : ''})`}. Discard them?`);
  if (ok) dirty.clear();
  return ok;
}
