import { useEffect, useState } from 'react';
import { LuCopy, LuTriangleAlert, LuCircleCheck, LuCircleSlash, LuPencilLine } from 'react-icons/lu';
import { call } from '../ipc';
import { Modal, useAction, useToast } from '../components/common';
import { cx } from '../format';

interface PreviewItem { source: string; dest: string; status: 'copy' | 'rename' | 'skip' | 'error'; detail: string }
interface Preview { projectName: string; mode: string; destDir: string; destAbs: string; operation: string; items: PreviewItem[] }

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
  const act = useAction();
  const toast = useToast();

  useEffect(() => {
    setPv(null);
    call<Preview>('files.importPreview', { projectId, paths, destDir: dir }).then(setPv, (e) => setErr(e.message));
  }, [projectId, paths, dir]);

  const n = pv?.items.filter((i) => i.status === 'copy' || i.status === 'rename').length || 0;
  const run = async () => {
    setBusy(true);
    const r: any = await act(() => call('files.import', { projectId, paths, destDir: dir }));
    setBusy(false);
    if (!r) return;
    onDone();
    onClose();
    const msg = `Imported ${r.imported.length} file${r.imported.length === 1 ? '' : 's'} into ${pv?.projectName}${dir ? ' / ' + dir : ''}` + (r.skipped.length ? `, skipped ${r.skipped.length}` : '') + (r.errors.length ? `, ${r.errors.length} errors` : '');
    if (r.batchId) {
      toast(msg, r.errors.length ? 'error' : 'success', {
        label: 'Undo import',
        onClick: async () => {
          const u: any = await act(() => call('files.undoImport', { id: r.batchId }));
          if (u) { toast(`Removed ${u.removed.length} imported copies${u.kept.length ? `, kept ${u.kept.length} (changed since or already gone)` : ''}`, 'success'); onDone(); }
        },
      });
    } else toast(msg, r.errors.length ? 'error' : 'info');
  };

  return (
    <Modal wide title="Import files" onClose={onClose} footer={<>
      <span className="small dim" style={{ marginRight: 'auto' }}>{pv ? `${n} to copy · ${pv.items.length - n} skipped` : ''}</span>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!pv || !n || busy} onClick={run}><LuCopy /> {busy ? 'Copying…' : `Copy ${n} file${n === 1 ? '' : 's'} into ${pv?.projectName || 'project'}`}</button>
    </>}>
      {err ? <div className="alert bad"><LuTriangleAlert /><div>{err}</div></div> : !pv ? <div className="muted">Checking…</div> : <>
        <div className="import-dest">
          <div><span className="dim small">Project</span><b>{pv.projectName}</b> <span className="pill">{pv.mode === 'linked' ? 'linked folder' : 'managed'}</span></div>
          <div><span className="dim small">Into folder</span>
            <div className="row" style={{ gap: 6 }}>
              <input className="input mono" style={{ width: 260 }} value={dirInput} placeholder="(project root)" onChange={(e) => setDirInput(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && setDir(dirInput.trim().replace(/^[\\/]+|[\\/]+$/g, ''))} />
              {dirInput !== dir && <button className="btn sm" onClick={() => setDir(dirInput.trim().replace(/^[\\/]+|[\\/]+$/g, ''))}>Apply</button>}
            </div>
          </div>
          <div><span className="dim small">Full location</span><span className="mono small" style={{ wordBreak: 'break-all' }}>{pv.destAbs}</span></div>
          <div><span className="dim small">Operation</span><span><b>Copy</b> — your original files are not moved or changed. Nothing is overwritten.</span></div>
        </div>
        <div className="import-list">
          {pv.items.map((it, i) => (
            <div key={i} className={cx('import-row', it.status)}>
              {it.status === 'copy' ? <LuCircleCheck /> : it.status === 'rename' ? <LuPencilLine /> : it.status === 'skip' ? <LuCircleSlash /> : <LuTriangleAlert />}
              <div className="grow" style={{ minWidth: 0 }}>
                <div className="mono small" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={it.source}>{it.source.split(/[\\/]/).pop()}</div>
                <div className="small dim">{it.status === 'copy' ? <>→ <span className="mono">{it.dest}</span></> : it.status === 'rename' ? <>→ <span className="mono">{it.dest}</span> · {it.detail}</> : it.detail}</div>
              </div>
              <span className="badge">{{ copy: 'copy', rename: 'renamed', skip: 'skipped', error: 'error' }[it.status]}</span>
            </div>
          ))}
        </div>
      </>}
    </Modal>
  );
}
