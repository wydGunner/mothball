import { useEffect, useState } from 'react';
import { LuRocket, LuPlay, LuSettings2, LuFolderOpen } from 'react-icons/lu';
import { call, pickFiles, pickFolder, type Project } from '../ipc';
import { Modal, useAction, useToast } from '../components/common';
import { ago, bytes, cx } from '../format';

interface DeployCfg { source: string; dest: string; exclude: string; clean: boolean; exe: string; args: string; cwd: string; last?: { at: number; copied: number; removed: number; errors: number } | null }
interface Preview { dest: string; source: string; copy: number; update: number; same: number; conflict: number; remove: number; keep: number; changes: { path: string; action: string; size?: number }[]; blocked?: boolean }

const DEPLOYABLE = new Set(['game', 'addon', 'software']);

const HINTS: Record<string, { dest: string; exe: string; args: string; note: string }> = {
  addon: { dest: 'C:\\Program Files (x86)\\World of Warcraft\\_retail_\\Interface\\AddOns\\MyAddon', exe: '', args: '', note: 'For WoW, deploy to Interface\\AddOns\\<addon folder name> — the folder name must match the .toc file name. Type /reload in game to pick up changes.' },
  game: { dest: 'D:\\Games\\MW2\\mods\\omen', exe: 'D:\\Games\\MW2\\iw4x.exe', args: '+set fs_game mods/omen', note: 'For IW4X, deploy to <game folder>\\mods\\<mod name> and launch iw4x.exe with +set fs_game mods/<mod name>.' },
  software: { dest: 'D:\\Tools\\MyTool', exe: '', args: '', note: '' },
};

/** Header buttons: Deploy (and Launch when a program is set). */
export function DeployButtons({ project }: { project: Project }) {
  const [cfg, setCfg] = useState<DeployCfg | null>(null);
  const [setup, setSetup] = useState(false);
  const [blocked, setBlocked] = useState<Preview | null>(null);
  const [busy, setBusy] = useState(false);
  const act = useAction();
  const toast = useToast();
  const load = () => call<DeployCfg>('deploy.get', { projectId: project.id }).then(setCfg).catch(() => setCfg(null));
  useEffect(() => { load(); /* eslint-disable-next-line */ }, [project.id]);
  if (!cfg) return null;
  const configured = !!cfg.dest;
  if (!configured && !DEPLOYABLE.has(project.category)) return null;

  const deploy = async (force = false, thenLaunch = false) => {
    setBusy(true);
    const r = await act(() => call<any>('deploy.run', { projectId: project.id, force }));
    setBusy(false);
    if (!r) return;
    if (r.blocked) { setBlocked(r); return; }
    setBlocked(null);
    const n = r.copied + r.removed;
    const deps: any[] = r.dependencies || [];
    const depFail = deps.filter((d) => d.result === 'failed');
    const depIn = deps.filter((d) => d.result === 'installed').map((d) => d.name);
    const depNote = depIn.length ? ` · put in place: ${depIn.join(', ')}` : '';
    if (r.errors.length) toast(`Deploy finished with ${r.errors.length} error(s):\n${r.errors.slice(0, 3).join('\n')}`, 'error');
    else if (depFail.length) toast(`Deployed, but ${depFail.map((d) => `${d.name} couldn't be put in place (${d.error})`).join('; ')}`, 'error');
    else toast(n || depIn.length ? `Deployed: ${r.copied} copied${r.removed ? `, ${r.removed} removed` : ''}${depNote}` : 'Already up to date — nothing to copy', 'success');
    window.dispatchEvent(new CustomEvent('pv:changed'));
    load();
    if (thenLaunch && !r.errors.length) await launch();
  };
  const launch = async () => { const r = await act(() => call('deploy.launch', { projectId: project.id })); if (r) toast('Started ' + cfg.exe.split(/[\\/]/).pop(), 'success'); };

  return (
    <>
      {configured ? (
        <div className="btngroup">
          <button className="btn" disabled={busy} onClick={() => deploy()} title={`Copy changed files to ${cfg.dest}${cfg.last ? ` (last deployed ${ago(cfg.last.at)})` : ''}`}><LuRocket /> {busy ? 'Deploying…' : 'Deploy'}</button>
          {cfg.exe && <button className="btn" disabled={busy} onClick={() => deploy(false, true)} title={`Deploy, then start ${cfg.exe.split(/[\\/]/).pop()}`}><LuPlay /> Deploy & launch</button>}
          <button className="btn icon" onClick={() => setSetup(true)} title="Deploy settings"><LuSettings2 /></button>
        </div>
      ) : (
        <button className="btn" onClick={() => setSetup(true)} title="Copy this project into a game or app folder in one click"><LuRocket /> Set up deploy</button>
      )}
      {setup && <DeploySetup project={project} cfg={cfg} onClose={() => setSetup(false)} onSaved={(c) => { setCfg(c); setSetup(false); }} />}
      {blocked && (
        <Modal title="Some files were changed in the destination" onClose={() => setBlocked(null)} footer={<>
          <button className="btn" onClick={() => setBlocked(null)}>Cancel</button>
          <button className="btn danger" onClick={() => deploy(true)}>Overwrite and deploy</button>
        </>}>
          <p className="muted" style={{ marginTop: 0 }}>These files in <span className="mono">{blocked.dest}</span> are different from what was last deployed, or weren’t put there by this app. Nothing was copied yet. Overwriting replaces them with the project’s versions — copy anything you changed there back into the project first if you want to keep it.</p>
          <ul className="small mono" style={{ maxHeight: 220, overflow: 'auto' }}>{blocked.changes.filter((c) => c.action === 'conflict').map((c) => <li key={c.path}>{c.path}</li>)}</ul>
        </Modal>
      )}
    </>
  );
}

function DeploySetup({ project, cfg, onClose, onSaved }: { project: Project; cfg: DeployCfg; onClose: () => void; onSaved: (c: DeployCfg) => void }) {
  const [f, setF] = useState<DeployCfg>(cfg);
  const [prev, setPrev] = useState<Preview | null>(null);
  const [prevErr, setPrevErr] = useState('');
  const act = useAction();
  const hint = HINTS[project.category] || HINTS.software;
  const save = async () => {
    const c = await act(() => call<DeployCfg>('deploy.set', { projectId: project.id, patch: f }));
    if (c) onSaved(c);
  };
  const preview = async () => {
    setPrev(null); setPrevErr('');
    const c = await act(() => call<DeployCfg>('deploy.set', { projectId: project.id, patch: f }));
    if (!c) return;
    try { setPrev(await call<Preview>('deploy.preview', { projectId: project.id })); } catch (e: any) { setPrevErr(e.message); }
  };
  return (
    <Modal wide title="Deploy & launch" onClose={onClose} footer={<>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn" disabled={!f.dest.trim()} onClick={preview}>Preview</button>
      <button className="btn primary" onClick={save}>Save</button>
    </>}>
      <p className="muted small" style={{ marginTop: 0 }}>Copies the project’s files into another folder — a game’s mod or addon folder, say — whenever you press Deploy. Only changed files are copied. Files someone changed in that folder are never overwritten without asking.{hint.note && <> {hint.note}</>}</p>
      <div className="field"><label>Deploy to</label>
        <div className="row"><input className="input mono" value={f.dest} placeholder={hint.dest} onChange={(e) => setF({ ...f, dest: e.target.value })} />
          <button className="btn" onClick={async () => { const p = await pickFolder('Folder to deploy into'); if (p) setF({ ...f, dest: p }); }}><LuFolderOpen /> Browse…</button></div>
      </div>
      <div className="grid2">
        <div className="field"><label>Folder to deploy <span className="dim">(inside the project)</span></label>
          <input className="input mono" value={f.source} placeholder="Whole project" onChange={(e) => setF({ ...f, source: e.target.value })} />
        </div>
        <div className="field"><label>Don’t copy <span className="dim">(one rule per line)</span></label>
          <textarea className="input mono" rows={2} value={f.exclude} placeholder={'docs/\n*.md'} onChange={(e) => setF({ ...f, exclude: e.target.value })} />
        </div>
      </div>
      <label className="row small"><input type="checkbox" checked={f.clean} onChange={(e) => setF({ ...f, clean: e.target.checked })} /> When a file is deleted from the project, remove the copy this app deployed (only if it wasn’t changed there)</label>
      <h3 style={{ marginTop: 16 }}>Launch <span className="small dim" style={{ fontWeight: 400 }}>(optional)</span></h3>
      <div className="field"><label>Program</label>
        <div className="row"><input className="input mono" value={f.exe} placeholder={hint.exe || 'C:\\Path\\to\\game.exe'} onChange={(e) => setF({ ...f, exe: e.target.value })} />
          <button className="btn" onClick={async () => { const p = await pickFiles('Program to start', [{ name: 'Programs', extensions: ['exe', 'bat', 'cmd'] }]); if (p[0]) setF({ ...f, exe: p[0] }); }}><LuFolderOpen /> Browse…</button></div>
      </div>
      <div className="grid2">
        <div className="field"><label>Arguments</label><input className="input mono" value={f.args} placeholder={hint.args} onChange={(e) => setF({ ...f, args: e.target.value })} /></div>
        <div className="field"><label>Start in</label><input className="input mono" value={f.cwd} placeholder="The program’s folder" onChange={(e) => setF({ ...f, cwd: e.target.value })} /></div>
      </div>
      {prevErr && <div className="alert bad">{prevErr}</div>}
      {prev && (
        <div className="card pad" style={{ marginTop: 8 }}>
          <div className="small"><b>{prev.copy}</b> new · <b>{prev.update}</b> updated · <b>{prev.same}</b> already up to date{prev.remove ? <> · <b>{prev.remove}</b> to remove</> : null}{prev.conflict ? <> · <b className="tl-del">{prev.conflict} changed in the destination</b></> : null}</div>
          {prev.changes.length > 0 && <div className="toclist" style={{ marginTop: 8 }}>
            {prev.changes.slice(0, 200).map((c) => <div key={c.path + c.action} className={cx('tocrow', c.action === 'conflict' && 'missing')}><span className={cx('tl-kind', c.action === 'copy' ? 'added' : c.action === 'update' ? 'modified' : 'missing')} style={{ width: 70 }}>{({ copy: 'New', update: 'Update', conflict: 'Changed', remove: 'Remove', keep: 'Keep' } as any)[c.action]}</span><span className="mono">{c.path}</span>{c.size != null && <span className="dim small right" style={{ whiteSpace: 'nowrap' }}>{bytes(c.size)}</span>}</div>)}
          </div>}
        </div>
      )}
    </Modal>
  );
}
