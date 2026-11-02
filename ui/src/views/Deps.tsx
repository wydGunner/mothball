// Files a project needs that live outside it (e.g. a modded iw4x.dll).
import { useState } from 'react';
import { LuPlus, LuPencil, LuTrash2, LuDownload, LuFolderSearch, LuUndo2 } from 'react-icons/lu';
import { call, pickFiles, revealPath, isTauri } from '../ipc';
import { Modal, useAction, useConfirm, useToast } from '../components/common';
import { bytes, cx } from '../format';

export interface Dep {
  id: string; name: string; fileName: string; sha256: string; size: number; placeAt: string; note: string; addedAt: number;
  stored: boolean; state: 'installed' | 'missing' | 'different' | 'kept' | 'unreachable' | 'unreadable';
  inPlaceBelongsTo?: string; canRestoreOriginal?: boolean;
}

const STATE: Record<Dep['state'], string> = {
  installed: 'In place',
  missing: 'Not in place',
  different: 'A different file is in place',
  kept: 'Kept in Mothball',
  unreachable: 'Location not reachable',
  unreadable: 'Can’t read the file in place',
};

export function depsSummary(deps: Dep[] | undefined): string | undefined {
  if (!deps) return undefined;
  if (!deps.length) return 'None attached';
  const off = deps.filter((d) => d.placeAt && d.state !== 'installed').length;
  return `${deps.length} file${deps.length === 1 ? '' : 's'}${off ? ` · ${off} not in place` : deps.some((d) => d.placeAt) ? ' · all in place' : ''}`;
}

export function DepsBody({ projectId, deps, onChanged }: { projectId: string; deps: Dep[] | undefined; onChanged: () => void }) {
  const [editing, setEditing] = useState<Dep | 'new' | null>(null);
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  const install = async (d: Dep) => {
    const r: any = await act(() => call('deps.install', { projectId, id: d.id }));
    const res = r?.[0];
    if (res?.result === 'failed') toast(`${d.name}: ${res.error}`, 'error');
    else if (res) toast(`${d.name} is in place`, 'success');
    onChanged();
  };
  return (
    <>
      {!deps?.length && <p className="small muted" style={{ marginTop: 0 }}>Attach files this project needs to work that live outside it — like the modded DLL a mod only runs with. Mothball keeps a copy, checks the right one is in place, and puts it there when you Deploy or Launch.</p>}
      {(deps || []).map((d) => (
        <div key={d.id} className="dep-row">
          <div className="row" style={{ gap: 8 }}>
            <span className={cx('pdot', d.state === 'installed' ? 'on' : d.state === 'kept' ? 'idle' : 'bad')} />
            <b className="grow" style={{ minWidth: 0 }}>{d.name}</b>
            {d.placeAt && d.state !== 'installed' && d.stored && <button className="btn sm" onClick={() => install(d)} title={`Copy Mothball's ${d.fileName} to ${d.placeAt}`}><LuDownload /> Put in place</button>}
            {isTauri && d.placeAt && <button className="btn ghost sm icon" title="Show in Explorer" onClick={() => act(() => revealPath(d.placeAt))}><LuFolderSearch /></button>}
            <button className="btn ghost sm icon" title="Edit" aria-label={`Edit ${d.name}`} onClick={() => setEditing(d)}><LuPencil /></button>
            <button className="btn ghost sm icon" title="Remove" aria-label={`Remove ${d.name}`} onClick={async () => {
              if (await confirm({ title: `Remove ${d.name}?`, confirm: 'Remove', danger: true, body: <>Mothball stops tracking it and deletes its stored copy (unless another project uses the same file). The file in the game folder is left where it is.</> })) {
                if (await act(() => call('deps.remove', { projectId, id: d.id }))) onChanged();
              }
            }}><LuTrash2 /></button>
          </div>
          <div className="small" style={{ paddingLeft: 17 }}>
            <span className={d.state === 'installed' || d.state === 'kept' ? 'muted' : 'bad-text'}>{STATE[d.state]}{d.state === 'different' && d.inPlaceBelongsTo ? ` (${d.inPlaceBelongsTo})` : ''}</span>
            {!d.stored && <span className="bad-text"> · Mothball’s copy is missing — attach the file again</span>}
            {d.placeAt && <div className="mono dim" style={{ wordBreak: 'break-all' }}>{d.placeAt}</div>}
            <div className="dim">{d.fileName} · {bytes(d.size)} · <span className="mono" title={d.sha256}>{d.sha256.slice(0, 12)}</span></div>
            {d.note && <div className="muted" style={{ whiteSpace: 'pre-wrap' }}>{d.note}</div>}
            {d.canRestoreOriginal && d.state === 'installed' && <button className="linklike small" style={{ marginTop: 2 }} onClick={async () => {
              if (await confirm({ title: 'Put the original file back?', confirm: 'Put it back', body: <>The file that was at <span className="mono">{d.placeAt}</span> before Mothball first replaced it goes back in place.</> })) {
                if (await act(() => call('deps.restoreOriginal', { projectId, id: d.id }), 'Original file put back')) onChanged();
              }
            }}><LuUndo2 /> Put the original file back</button>}
          </div>
        </div>
      ))}
      <button className="btn sm" style={{ marginTop: 8 }} onClick={() => setEditing('new')}><LuPlus /> Attach dependency…</button>
      {editing && <DepDialog projectId={projectId} dep={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onSaved={() => { setEditing(null); onChanged(); }} />}
    </>
  );
}

function DepDialog({ projectId, dep, onClose, onSaved }: { projectId: string; dep: Dep | null; onClose: () => void; onSaved: () => void }) {
  const [path, setPath] = useState('');
  const [name, setName] = useState(dep?.name || '');
  const [placeAt, setPlaceAt] = useState(dep?.placeAt ?? '');
  const [note, setNote] = useState(dep?.note || '');
  const [busy, setBusy] = useState(false);
  const act = useAction();
  const browse = async () => {
    const f = (await pickFiles('Choose the file this project needs'))[0];
    if (!f) return;
    setPath(f);
    if (!name) setName(f.split(/[\\/]/).pop() || '');
    if (!placeAt) setPlaceAt(f); // usually it's already where it has to be
  };
  const save = async () => {
    setBusy(true);
    const r = dep
      ? await act(() => call('deps.update', { projectId, id: dep.id, patch: { name, placeAt, note } }))
      : await act(() => call('deps.add', { projectId, path, name, placeAt, note }));
    setBusy(false);
    if (r) onSaved();
  };
  return (
    <Modal title={dep ? `Edit ${dep.name}` : 'Attach a dependency'} onClose={onClose} footer={<>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={busy || (!dep && !path.trim())} onClick={save}>{busy ? 'Saving…' : dep ? 'Save' : 'Attach'}</button>
    </>}>
      {!dep && <div className="field"><label>File</label>
        <div className="row"><input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} placeholder="D:\Games\MW2\iw4x.dll" /><button className="btn" onClick={browse}>Browse…</button></div>
        <div className="small dim" style={{ marginTop: 4 }}>Mothball keeps its own copy of this exact file, so it can't get lost or mixed up with another version.</div>
      </div>}
      <div className="field"><label>Name</label><input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="iw4x.dll (modded)" /></div>
      <div className="field"><label>Where it has to be for the project to work</label>
        <input className="input mono" value={placeAt} onChange={(e) => setPlaceAt(e.target.value)} placeholder="D:\Games\MW2\iw4x.dll — leave empty to just keep it" />
        <div className="small dim" style={{ marginTop: 4 }}>Deploy and Launch put it here first. If a file Mothball doesn't know is already there, it's kept so you can put it back.</div>
      </div>
      <div className="field"><label>Note</label><textarea className="input" rows={2} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Old IW4X build; the newer one breaks the bot menu" /></div>
    </Modal>
  );
}
