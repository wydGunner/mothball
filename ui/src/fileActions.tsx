// File management actions shared by the Files tab and global search, so both
// use the same dialogs, validation and backend commands.
import { useCallback } from 'react';
import { LuExternalLink, LuFolderSearch, LuCopy, LuPencilLine, LuTrash2, LuCode, LuLocateFixed, LuArchiveRestore, LuFileText } from 'react-icons/lu';
import { call, isTauri, openPath, revealPath } from './ipc';
import { type MenuItem, useAction, useConfirm, usePrompt, useToast } from './components/common';

export interface FileRef {
  id: string;
  name: string;
  path: string;          // project-relative, as indexed
  absPath?: string;      // real location on disk
  projectId: string;
  projectName?: string;
  state: string;         // present | missing | deleted
  editable?: boolean;
}

const BAD_CHARS = /[\\/:*?"<>|\u0000-\u001f]/;
const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;
export function fileNameError(name: string, current: string): string | null {
  const n = name.trim();
  if (!n || n === '.' || n === '..') return 'Enter a file name';
  if (BAD_CHARS.test(n)) return 'File names can’t contain \\ / : * ? " < > |';
  if (RESERVED.test(n)) return `“${n}” is a reserved name on Windows`;
  if (n === current) return 'That’s the current name';
  return null;
}

export const TEXT_EDITABLE = (kind: string, ext: string, size: number) =>
  size <= 8 * 1024 * 1024 && (kind === 'code' || kind === 'config' || ['md', 'markdown', 'txt', 'csv', 'log', 'gcode', 'scad', 'meta', 'asmdef', 'gitignore', 'env', 'svg', 'shader', 'hlsl', 'glsl', 'menu'].includes(ext));

export function useFileActions() {
  const act = useAction();
  const confirm = useConfirm();
  const prompt = usePrompt();
  const toast = useToast();

  const copy = useCallback(async (text: string, what: string) => {
    try { await navigator.clipboard.writeText(text); toast(`${what} copied`); } catch { toast('Could not access the clipboard', 'error'); }
  }, [toast]);

  const rename = useCallback(async (f: FileRef): Promise<boolean> => {
    const name = await prompt({ title: 'Rename file', label: `New name for ${f.path}`, value: f.name, confirm: 'Rename', selectUntilDot: true, validate: (v) => fileNameError(v, f.name) });
    if (!name) return false;
    const r = await act(() => call('files.rename', { id: f.id, name }), `Renamed to ${name.trim()}`);
    return !!r;
  }, [act, prompt]);

  const remove = useCallback(async (files: FileRef[]): Promise<boolean> => {
    const live = files.filter((f) => f.state === 'present');
    if (!live.length) return false;
    const one = live.length === 1;
    const ok = await confirm({
      title: one ? `Delete “${live[0].name}”?` : `Delete ${live.length} files?`,
      confirm: 'Delete', danger: true,
      body: <>
        {one ? <span className="mono">{live[0].path}</span> : <ul style={{ margin: '0 0 8px', paddingLeft: 18, maxHeight: 160, overflow: 'auto' }}>{live.map((f) => <li key={f.id} className="mono">{f.path}</li>)}</ul>}
        {one ? ' will be' : 'These will be'} moved to the {isTauri ? 'Windows Recycle Bin' : 'trash'}. Project Vault keeps a verified copy and {one ? 'its' : 'their'} notes, so you can restore {one ? 'it' : 'them'} from the file page (Files → Filter → Deleted).
      </>,
    });
    if (!ok) return false;
    const r: any = await act(() => call('files.delete', { ids: live.map((f) => f.id) }));
    if (!r) return false;
    if (r.errors.length) toast(r.errors.join('\n'), 'error');
    if (r.deleted.length) toast(r.deleted.length === 1 ? `Deleted ${r.deleted[0]}` : `Deleted ${r.deleted.length} files`, 'success');
    return r.deleted.length > 0;
  }, [act, confirm, toast]);

  const restore = useCallback(async (f: FileRef) => !!(await act(() => call('files.restoreDeleted', { id: f.id }), `Restored ${f.path}`)), [act]);

  /**
   * Context-menu entries for a file. `selection` = all selected files when the
   * menu is opened on a multi-selection (bulk delete).
   */
  const menuItems = useCallback((f: FileRef, o: {
    selection?: FileRef[];
    onOpen?: () => void;      // show the file in Project Vault
    onEdit?: () => void;      // open in the built-in editor
    onReveal?: () => void;    // go to project → Files → select
    onChanged: (kind: 'renamed' | 'deleted' | 'restored') => void;
    extra?: MenuItem[];
  }): MenuItem[] => {
    const sel = o.selection && o.selection.length > 1 ? o.selection : [f];
    const present = f.state === 'present';
    const items: MenuItem[] = [];
    if (o.onEdit && f.editable && present) items.push({ label: 'Edit in Project Vault', icon: <LuCode />, onClick: o.onEdit });
    if (o.onOpen) items.push({ label: 'Open file details', icon: <LuFileText />, onClick: o.onOpen });
    if (o.onReveal) items.push({ label: 'Reveal in project', icon: <LuLocateFixed />, onClick: o.onReveal });
    items.push(
      { label: 'Open with default app', icon: <LuExternalLink />, disabled: !isTauri || !present || !f.absPath, onClick: () => act(() => openPath(f.absPath!)) },
      { label: 'Show in Windows Explorer', icon: <LuFolderSearch />, disabled: !isTauri || !present || !f.absPath, onClick: () => act(() => revealPath(f.absPath!)) },
      { sep: true, label: '' },
      { label: 'Copy full path', icon: <LuCopy />, disabled: !f.absPath, onClick: () => copy(f.absPath!, 'Full path') },
      { label: 'Copy relative path', icon: <LuCopy />, onClick: () => copy(f.path, 'Relative path') },
    );
    if (o.extra?.length) items.push({ sep: true, label: '' }, ...o.extra);
    items.push({ sep: true, label: '' });
    if (f.state === 'deleted') {
      items.push({ label: 'Restore deleted file', icon: <LuArchiveRestore />, onClick: async () => { if (await restore(f)) o.onChanged('restored'); } });
    } else {
      items.push(
        { label: 'Rename…', icon: <LuPencilLine />, disabled: !present || sel.length > 1, onClick: async () => { if (await rename(f)) o.onChanged('renamed'); } },
        { label: sel.length > 1 ? `Delete ${sel.filter((x) => x.state === 'present').length} files…` : 'Delete…', icon: <LuTrash2 />, danger: true, disabled: !sel.some((x) => x.state === 'present'), onClick: async () => { if (await remove(sel)) o.onChanged('deleted'); } },
      );
    }
    return items;
  }, [act, copy, rename, remove, restore]);

  return { menuItems, rename, remove, restore, copy };
}
