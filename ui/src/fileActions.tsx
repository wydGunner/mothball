// File management actions shared by the Files tab and global search, so both
// use the same dialogs, validation and backend commands.
import { createContext, useCallback, useContext, useState, type ReactNode } from 'react';
import { LuExternalLink, LuFolderSearch, LuCopy, LuPencilLine, LuTrash2, LuCode, LuLocateFixed, LuArchiveRestore, LuFileText, LuEyeOff, LuFolderInput, LuHistory } from 'react-icons/lu';
import { call, isTauri, openPath, revealPath, type Project } from './ipc';
import { Modal, type MenuItem, useAction, useConfirm, usePrompt, useToast } from './components/common';

// ---- "Move to another project" dialog (promise-based, mounted once in main.tsx)
type MoveReq = { files: FileRef[]; resolve: (r: { projectId: string; dir: string } | null) => void };
const MoveCtx = createContext<(files: FileRef[]) => Promise<{ projectId: string; dir: string } | null>>(async () => null);
export function FileDialogsProvider({ children }: { children: ReactNode }) {
  const [req, setReq] = useState<MoveReq | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [target, setTarget] = useState('');
  const [dir, setDir] = useState('');
  const ask = useCallback((files: FileRef[]) => new Promise<{ projectId: string; dir: string } | null>((resolve) => {
    call<Project[]>('projects.list').then((ps) => { setProjects(ps.filter((p) => p.id !== files[0]?.projectId)); setTarget(''); setDir(''); setReq({ files, resolve }); });
  }), []);
  const done = (v: { projectId: string; dir: string } | null) => { req?.resolve(v); setReq(null); };
  const tp = projects.find((p) => p.id === target);
  return (
    <MoveCtx.Provider value={ask}>
      {children}
      {req && (
        <Modal title={req.files.length === 1 ? `Move “${req.files[0].name}” to another project` : `Move ${req.files.length} files to another project`} onClose={() => done(null)}
          footer={<><button className="btn" onClick={() => done(null)}>Cancel</button><button className="btn primary" disabled={!target} onClick={() => done({ projectId: target, dir: dir.trim().replace(/^[\\/]+|[\\/]+$/g, '') })}>Move</button></>}>
          <div className="field"><label>Destination project</label>
            <select className="input" value={target} onChange={(e) => setTarget(e.target.value)} autoFocus>
              <option value="">Choose a project…</option>{projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
            </select>
          </div>
          <div className="field"><label>Folder inside that project (optional)</label><input className="input mono" value={dir} onChange={(e) => setDir(e.target.value)} placeholder="e.g. scripts/mp — empty = project root" /></div>
          {tp && <p className="small muted" style={{ margin: 0 }}>
            The file{req.files.length > 1 ? 's are' : ' is'} copied to <span className="mono">{tp.rootPath}{dir ? '\\' + dir.replace(/\//g, '\\') : ''}</span> and checked, then the original
            {req.files.length > 1 ? 's go' : ' goes'} to the Recycle Bin. Notes, tags and stored versions move along; links to notes/tasks in the old project are removed.
          </p>}
        </Modal>
      )}
    </MoveCtx.Provider>
  );
}

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
  const askMove = useContext(MoveCtx);

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
        <p style={{ marginTop: 0 }}><b>This deletes the actual file{one ? '' : 's'} from your disk</b> (moved to the {isTauri ? 'Windows Recycle Bin' : 'trash'}).
          {' '}To only stop tracking a file in Mothball, use “Remove from Mothball” instead.</p>
        <div className="delete-list">
          {live.map((f) => <div key={f.id}><b>{f.name}</b><div className="mono small dim" style={{ wordBreak: 'break-all' }}>{f.absPath || f.path}</div></div>)}
        </div>
        <p className="small" style={{ marginBottom: 0 }}>Mothball keeps a verified copy and the notes, so you can restore {one ? 'it' : 'them'} from Files → Filter → Deleted.</p>
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

  /** Stop tracking — the file stays on disk. */
  const untrack = useCallback(async (files: FileRef[]): Promise<boolean> => {
    const one = files.length === 1;
    const ok = await confirm({
      title: one ? `Remove “${files[0].name}” from Mothball?` : `Remove ${files.length} files from Mothball?`,
      confirm: 'Remove from Mothball',
      body: <>
        <p style={{ marginTop: 0 }}><b>The file{one ? ' stays' : 's stay'} on your disk, untouched.</b> Mothball stops tracking {one ? 'it' : 'them'}: {one ? 'its' : 'their'} notes, tags, links and stored versions in Mothball are removed, and a rule is added so future scans skip {one ? 'it' : 'them'} (you can remove the rule under Edit → Files &amp; versions).</p>
        <div className="delete-list">{files.map((f) => <div key={f.id} className="mono small">{f.path}</div>)}</div>
      </>,
    });
    if (!ok) return false;
    return !!(await act(() => call('files.stopTracking', { ids: files.map((f) => f.id) }), one ? `No longer tracking ${files[0].path}` : `No longer tracking ${files.length} files`));
  }, [act, confirm]);

  const move = useCallback(async (files: FileRef[]): Promise<boolean> => {
    const live = files.filter((f) => f.state === 'present');
    if (!live.length) return false;
    const dest = await askMove(live);
    if (!dest) return false;
    const r: any = await act(() => call('files.moveToProject', { ids: live.map((f) => f.id), targetProjectId: dest.projectId, destDir: dest.dir }));
    if (!r) return false;
    if (r.errors.length) toast(r.errors.join('\n'), 'error');
    if (r.moved.length) toast(`Moved ${r.moved.length} file${r.moved.length === 1 ? '' : 's'}`, 'success');
    return r.moved.length > 0;
  }, [act, askMove, toast]);

  /**
   * Context-menu entries for a file. `selection` = all selected files when the
   * menu is opened on a multi-selection (bulk delete).
   */
  const menuItems = useCallback((f: FileRef, o: {
    selection?: FileRef[];
    onOpen?: () => void;      // show the file in Mothball
    onEdit?: () => void;      // open in the built-in editor
    onReveal?: () => void;    // go to project → Files → select
    onHistory?: () => void;   // open the file's version history
    onChanged: (kind: 'renamed' | 'deleted' | 'restored') => void;
    extra?: MenuItem[];
  }): MenuItem[] => {
    const sel = o.selection && o.selection.length > 1 ? o.selection : [f];
    const present = f.state === 'present';
    const items: MenuItem[] = [];
    if (o.onEdit && f.editable && present) items.push({ label: 'Edit in Mothball', icon: <LuCode />, onClick: o.onEdit });
    if (o.onOpen) items.push({ label: 'Open file details', icon: <LuFileText />, onClick: o.onOpen });
    if (o.onReveal) items.push({ label: 'Reveal in project', icon: <LuLocateFixed />, onClick: o.onReveal });
    if (o.onHistory) items.push({ label: 'Version history', icon: <LuHistory />, onClick: o.onHistory });
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
        { label: 'Move to another project…', icon: <LuFolderInput />, disabled: !sel.some((x) => x.state === 'present'), onClick: async () => { if (await move(sel)) o.onChanged('deleted'); } },
        { sep: true, label: '' },
        { label: sel.length > 1 ? `Remove ${sel.length} from Mothball…` : 'Remove from Mothball…', icon: <LuEyeOff />, onClick: async () => { if (await untrack(sel)) o.onChanged('deleted'); } },
        { label: sel.length > 1 ? `Delete ${sel.filter((x) => x.state === 'present').length} files from disk…` : 'Delete file from disk…', icon: <LuTrash2 />, danger: true, disabled: !sel.some((x) => x.state === 'present'), onClick: async () => { if (await remove(sel)) o.onChanged('deleted'); } },
      );
    }
    return items;
  }, [act, copy, rename, remove, restore, untrack, move]);

  return { menuItems, rename, remove, restore, copy, untrack, move };
}
