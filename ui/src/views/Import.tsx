import { useEffect, useState } from 'react';
import { LuCopy, LuTriangleAlert, LuCircleCheck, LuCircleSlash, LuPencilLine, LuMoveRight, LuFolderOpen, LuRefreshCw, LuPlus } from 'react-icons/lu';
import { call, pickFolder } from '../ipc';
import { Modal, useAction, useToast } from '../components/common';
import { cx } from '../format';

interface PreviewItem { source: string; dest: string; status: 'copy' | 'rename' | 'skip' | 'error'; detail: string }
interface Preview { projectName: string; mode: string; destDir: string; destAbs: string; root: string; fileCount: number; operation: string; items: PreviewItem[] }
type Op = 'copy' | 'move' | 'inplace' | 'update';
interface UpdatePreview { strip: string; changed: number; new: number; same: number; ignored: number; whole: boolean; items: { path: string; status: 'changed' | 'new' | 'same' }[]; notInDrop: string[] }

/** The folder that contains every source (for “work on them where they are”). */
function commonParent(paths: string[]): string {
  const split = (p: string) => p.replace(/[\\/]+$/, '').split(/[\\/]/);
  const parts = paths.map((p) => split(p).slice(0, -1));
  if (!parts.length) return '';
  const out: string[] = [];
  for (let i = 0; i < parts[0].length; i++) {
    const seg = parts[0][i];
    if (parts.every((x) => x[i] !== undefined && x[i].toLowerCase() === seg.toLowerCase())) out.push(seg); else break;
  }
  const sep = paths[0].includes('\\') ? '\\' : '/';
  return out.join(sep) || (sep === '/' ? '/' : '');
}

/**
 * Confirms an import before anything touches the disk: shows the destination
 * project and folder, that files are COPIED, and per-file conflicts. After a
 * successful import, the toast offers Undo.
 */
export function ImportDialog({ projectId, paths, initialDir, onClose, onDone }: { projectId: string; paths: string[]; initialDir: string; onClose: () => void; onDone: () => void }) {
  const [dir, setDir] = useState(initialDir);
  const [dirInput, setDirInput] = useState(initialDir);
  const [pv, setPv] = useState<Preview | null>(null);
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [op, setOp] = useState<Op>('copy');
  // "Update the project from this": a newer copy of the project (or some of its files)
  const [up, setUp] = useState<UpdatePreview | null>(null);
  const [removeMissing, setRemoveMissing] = useState(false);
  const [showSame, setShowSame] = useState(false);
  useEffect(() => {
    call<UpdatePreview>('files.updatePreview', { projectId, paths }).then((u) => {
      setUp(u);
      // it matches files already in the project: updating is almost certainly what's wanted
      // (a whole folder or zip, or loose files that are all already in it)
      if (u.changed + u.same > 0 && (u.whole || u.new === 0)) setOp('update');
    }, () => setUp(null));
  }, [projectId, paths]);
  const nUp = up ? up.changed + up.new + (removeMissing ? up.notInDrop.length : 0) : 0;
  const runUpdate = async () => {
    setBusy(true);
    const r: any = await act(() => call('files.update', { projectId, paths, removeMissing }));
    setBusy(false);
    if (!r) return;
    onDone();
    onClose();
    const parts = [r.updated.length && `replaced ${r.updated.length}`, r.added.length && `added ${r.added.length}`, r.removed.length && `removed ${r.removed.length}`].filter(Boolean).join(', ');
    const msg = `Updated ${pv?.projectName || 'the project'}: ${parts || 'nothing to change'}${r.errors.length ? `. ${r.errors.length} couldn't be updated: ${r.errors.map((e: any) => (e.path ? e.path + ': ' : '') + e.error).join('; ')}` : ''}`;
    if (r.entryId) toast(msg, r.errors.length ? 'error' : 'success', {
      label: 'Undo',
      onClick: async () => { const u: any = await act(() => call('timeline.undo', { id: r.entryId })); if (u) { toast(`Undid the update (${u.done.length} file${u.done.length === 1 ? '' : 's'}). A snapshot from just before is under Backups.`, 'success'); onDone(); } },
    });
    else toast(msg, r.errors.length ? 'error' : 'info');
  };
  const [inplaceDir, setInplaceDir] = useState(() => commonParent(paths));
  const act = useAction();
  const toast = useToast();

  useEffect(() => {
    setPv(null);
    call<Preview>('files.importPreview', { projectId, paths, destDir: dir }).then(setPv, (e) => setErr(e.message));
  }, [projectId, paths, dir]);

  const n = pv?.items.filter((i) => i.status === 'copy' || i.status === 'rename').length || 0;
  const useInPlace = async () => {
    setBusy(true);
    const r: any = await act(() => call('projects.useFolder', { id: projectId, path: inplaceDir }));
    setBusy(false);
    if (!r) return;
    onDone();
    onClose();
    toast(`${r.name} now works on ${r.rootPath} directly${r.copied ? ` — ${r.copied} file(s) from app storage were copied there` : ''}${r.renamed?.length ? ` (${r.renamed.length} renamed to avoid overwriting)` : ''}. The old copy in app storage was left as it was.`, 'success');
  };
  const run = async () => {
    if (op === 'update') return runUpdate();
    if (op === 'inplace') return useInPlace();
    setBusy(true);
    const r: any = await act(() => call('files.import', { projectId, paths, destDir: dir, operation: op }));
    setBusy(false);
    if (!r) return;
    onDone();
    onClose();
    const msg = `${op === 'move' ? 'Moved' : 'Imported'} ${r.imported.length} file${r.imported.length === 1 ? '' : 's'} into ${pv?.projectName}${dir ? ' / ' + dir : ''}` + (r.skipped.length ? `, skipped ${r.skipped.length}` : '') + (r.errors.length ? `, ${r.errors.length} errors` : '');
    if (r.batchId) {
      toast(msg, r.errors.length ? 'error' : 'success', {
        label: 'Undo import',
        onClick: async () => {
          const u: any = await act(() => call('files.undoImport', { id: r.batchId }));
          if (u) { toast(`${op === 'move' ? `Moved ${u.removed.length} file(s) back` : `Removed ${u.removed.length} imported copies`}${u.kept.length ? `, kept ${u.kept.length} (changed since or already gone)` : ''}`, 'success'); onDone(); }
        },
      });
    } else toast(msg, r.errors.length ? 'error' : 'info');
  };

  return (
    <Modal wide title={op === 'update' ? 'Update the project' : 'Import files'} onClose={onClose} footer={<>
      <span className="small dim" style={{ marginRight: 'auto' }}>{op === 'update' ? (up ? `${up.changed} replaced · ${up.new} added · ${up.same} unchanged` : '') : pv && op !== 'inplace' ? `${n} to ${op} · ${pv.items.length - n} skipped` : ''}</span>
      <button className="btn" onClick={onClose}>Cancel</button>
      {op === 'update'
        ? <button className="btn primary" disabled={!up || !nUp || busy} onClick={run}><LuRefreshCw /> {busy ? 'Updating…' : nUp ? `Update ${nUp} file${nUp === 1 ? '' : 's'}` : 'Already up to date'}</button>
        : op === 'inplace'
        ? <button className="btn primary" disabled={!pv || busy || !inplaceDir} onClick={run}><LuFolderOpen /> {busy ? 'Switching…' : 'Use this folder'}</button>
        : <button className="btn primary" disabled={!pv || !n || busy} onClick={run}>{op === 'move' ? <LuMoveRight /> : <LuCopy />} {busy ? (op === 'move' ? 'Moving…' : 'Copying…') : `${op === 'move' ? 'Move' : 'Copy'} ${n} file${n === 1 ? '' : 's'} into ${pv?.projectName || 'project'}`}</button>}
    </>}>
      {err ? <div className="alert bad"><LuTriangleAlert /><div>{err}</div></div> : !pv ? <div className="muted">Checking…</div> : <>
        <div className="import-dest">
          <div><span className="dim small">Project</span><span className="row" style={{ gap: 8 }}><b>{pv.projectName}</b><span className="pill" title={pv.root}>{pv.mode === 'linked' ? 'works on your folder' : 'files kept in app storage'}</span></span></div>
          {op !== 'inplace' && op !== 'update' && <><div><span className="dim small">Into folder</span>
            <div className="row" style={{ gap: 6 }}>
              <input className="input mono" style={{ width: 260 }} value={dirInput} placeholder="(project root)" onChange={(e) => setDirInput(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && setDir(dirInput.trim().replace(/^[\\/]+|[\\/]+$/g, ''))} />
              {dirInput !== dir && <button className="btn sm" onClick={() => setDir(dirInput.trim().replace(/^[\\/]+|[\\/]+$/g, ''))}>Apply</button>}
            </div>
          </div>
          <div><span className="dim small">Full location</span><span className="mono small" style={{ wordBreak: 'break-all' }}>{pv.destAbs}</span></div></>}
        </div>
        <div className="opchoice" role="radiogroup" aria-label="What to do with the files">
          {up && up.changed + up.same > 0 && <label className={cx('opcard', op === 'update' && 'on')}><input type="radio" name="importop" checked={op === 'update'} onChange={() => setOp('update')} />
            <div><b><LuRefreshCw /> Update the project's files</b><div className="small dim">A newer copy of this project: files that changed replace the project's, new ones are added. What's replaced is kept, and the update can be undone.</div></div></label>}
          <label className={cx('opcard', op === 'copy' && 'on')}><input type="radio" name="importop" checked={op === 'copy'} onChange={() => setOp('copy')} />
            <div><b><LuCopy /> Copy into the project</b><div className="small dim">Your originals stay where they are, unchanged. Edits in Mothball change the copy.</div></div></label>
          <label className={cx('opcard', op === 'move' && 'on')}><input type="radio" name="importop" checked={op === 'move'} onChange={() => setOp('move')} />
            <div><b><LuMoveRight /> Move into the project</b><div className="small dim">The files leave their current folder, so there's only one copy. Undo moves them back.</div></div></label>
          <label className={cx('opcard', op === 'inplace' && 'on', pv.mode !== 'managed' && 'disabled')}><input type="radio" name="importop" checked={op === 'inplace'} disabled={pv.mode !== 'managed'} onChange={() => setOp('inplace')} />
            <div><b><LuFolderOpen /> Edit them where they are</b><div className="small dim">{pv.mode === 'managed'
              ? 'The project starts working on that folder directly — nothing is copied, and you edit the real files.'
              : <>This project already works directly on <span className="mono">{pv.root}</span>. Files outside it need to be copied or moved in.</>}</div></div></label>
        </div>
        {op === 'inplace' && <div className="import-dest">
          <div><span className="dim small">Project folder</span>
            <div className="row" style={{ gap: 6, minWidth: 0 }}>
              <span className="mono small grow" style={{ wordBreak: 'break-all' }}>{inplaceDir || '—'}</span>
              <button className="btn sm" onClick={async () => { const f = await pickFolder('Folder this project should work on'); if (f) setInplaceDir(f); }}>Choose…</button>
            </div></div>
          <div><span className="dim small">What happens</span><span className="small">
            {pv.projectName} becomes a <b>linked</b> project on this folder: every file in it shows up in Mothball and is edited in place.
            {pv.fileCount > 0 && <> The {pv.fileCount} file(s) already in the project are copied there first (never overwriting — a different file with the same name gets a new name). The old copy in app storage is left alone.</>}
            {' '}Make sure the folder contains only this project.
          </span></div>
        </div>}
        {op === 'update' && up && <div className="update-plan">
          <div className="small">{up.strip ? <>Matched <span className="mono">{up.strip}/</span> to the project folder.</> : 'Matched to the project folder.'}{up.ignored ? <span className="dim"> {up.ignored} file(s) the project ignores were left out.</span> : null}</div>
          {up.changed + up.new === 0 && <div className="small dim" style={{ marginTop: 6 }}>Everything in the drop is already the same as the project.</div>}
          <div className="import-list">
            {up.items.filter((i) => i.status !== 'same').map((i) => (
              <div key={i.path} className={cx('import-row', i.status === 'new' ? 'copy' : 'rename')}>
                {i.status === 'new' ? <LuPlus /> : <LuRefreshCw />}<span className="mono grow">{i.path}</span><span className="small dim">{i.status === 'new' ? 'new: added' : 'changed: replaced'}</span>
              </div>
            ))}
            {up.same > 0 && <button className="btn ghost sm" onClick={() => setShowSame(!showSame)}>{showSame ? 'Hide' : 'Show'} {up.same} unchanged</button>}
            {showSame && up.items.filter((i) => i.status === 'same').map((i) => <div key={i.path} className="import-row skip"><LuCircleSlash /><span className="mono grow">{i.path}</span><span className="small dim">same: untouched</span></div>)}
          </div>
          {up.notInDrop.length > 0 && <div className="update-missing">
            <label className="row small" style={{ gap: 6 }}><input type="checkbox" checked={removeMissing} onChange={(e) => setRemoveMissing(e.target.checked)} />
              <span>Also remove the <b>{up.notInDrop.length}</b> project file{up.notInDrop.length === 1 ? '' : 's'} that aren't in it <span className="dim">(to the Recycle Bin, with a copy kept). Leave this off if the download is only part of the project.</span></span></label>
            {removeMissing && <div className="import-list">{up.notInDrop.map((p) => <div key={p} className="import-row error"><LuTriangleAlert /><span className="mono grow">{p}</span><span className="small dim">removed</span></div>)}</div>}
          </div>}
        </div>}
        {op !== 'inplace' && op !== 'update' && <div className="import-list">
          {pv.items.map((it, i) => (
            <div key={i} className={cx('import-row', it.status)}>
              {it.status === 'copy' ? <LuCircleCheck /> : it.status === 'rename' ? <LuPencilLine /> : it.status === 'skip' ? <LuCircleSlash /> : <LuTriangleAlert />}
              <div className="grow" style={{ minWidth: 0 }}>
                <div className="mono small" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={it.source}>{it.source.split(/[\\/]/).pop()}</div>
                <div className="small dim">{it.status === 'copy' ? <>→ <span className="mono">{it.dest}</span></> : it.status === 'rename' ? <>→ <span className="mono">{it.dest}</span> · {it.detail}</> : it.detail}</div>
              </div>
              <span className="badge">{{ copy: op, rename: 'renamed', skip: 'skipped', error: 'error' }[it.status]}</span>
            </div>
          ))}
        </div>}
      </>}
    </Modal>
  );
}
