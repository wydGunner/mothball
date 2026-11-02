import { useEffect, useState } from 'react';
import { LuArrowLeft, LuArrowRight, LuFolderOpen, LuFolderPlus, LuArchiveRestore, LuX, LuFileText, LuSearch, LuHistory } from 'react-icons/lu';
import { call, pickFolder, type Project } from '../ipc';
import { ProjectIcon, useAction } from '../components/common';
import { CATEGORY_LABELS, cx, hiddenTypes, typeChoices } from '../format';
import type { Nav } from '../main';

/** Shown instead of an empty dashboard: add the first project in two steps. */
export function Welcome({ nav }: { nav: Nav }) {
  const [mode, setMode] = useState<null | 'linked' | 'managed'>(null);
  const [path, setPath] = useState('');
  const [name, setName] = useState('');
  const [category, setCategory] = useState('general');
  const [guess, setGuess] = useState<null | { category: string; reason: string }>(null);
  const [busy, setBusy] = useState(false);
  const act = useAction();

  const pick = async () => {
    const p = await pickFolder('Choose a project folder you already have');
    if (!p) return;
    setPath(p);
    setName(p.split(/[\\/]/).filter(Boolean).pop() || '');
    setMode('linked');
  };
  useEffect(() => {
    if (mode !== 'linked' || !path) { setGuess(null); return; }
    let live = true;
    call<{ category: string; reason: string } | null>('profiles.guess', { path }).then((g) => {
      if (!live) return;
      const ok = g && !hiddenTypes().includes(g.category) ? g : null;
      setGuess(ok);
      if (ok) setCategory(ok.category);
    }).catch(() => {});
    return () => { live = false; };
  }, [mode, path]);

  const create = async () => {
    setBusy(true);
    const p = await act(() => call<Project>('projects.create', { name: name.trim(), description: '', category, status: 'Active', linkPath: mode === 'linked' ? path : null }));
    if (p) {
      await act(() => call('files.scan', { projectId: p.id }));
      nav.refreshProjects();
      nav.go({ view: 'project', id: p.id, tab: 'files' });
    }
    setBusy(false);
  };
  const restore = () => { try { localStorage.setItem('pv.settingsSection', 'restore'); } catch { /* ignore */ } nav.go({ view: 'settings' }); };

  return (
    <div className="page welcome">
      <div className="wl-hero">
        <h1>Every project, and what every file in it is for.</h1>
        <p>Add the folders you already work in. Nothing is moved or renamed: the app indexes them where they are, keeps earlier versions of the files that matter, and lets you note down what each file does, so it still makes sense months from now.</p>
      </div>

      {!mode && (
        <>
          <div className="wl-choices">
            <button className="wl-choice" onClick={pick}>
              <LuFolderOpen />
              <b>Add a folder I already have</b>
              <span>A mod, an edit, a print, a CAD part — any folder. It stays where it is.</span>
              <LuArrowRight className="go" />
            </button>
            <button className="wl-choice" onClick={() => { setMode('managed'); setName(''); setCategory('general'); }}>
              <LuFolderPlus />
              <b>Start a new project</b>
              <span>The app makes a folder for it and you add files as you go.</span>
              <LuArrowRight className="go" />
            </button>
          </div>
          <div className="wl-links">
            <button className="linklike" onClick={restore}><LuArchiveRestore /> Restore from a backup</button>
          </div>
        </>
      )}

      {mode && (
        <div className="card pad wl-form">
          <button className="btn ghost sm" onClick={() => setMode(null)}><LuArrowLeft /> Back</button>
          {mode === 'linked' && <div className="small muted mono wl-path" title={path}>{path}</div>}
          <div className="field"><label>Name</label>
            <input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="My project" onKeyDown={(e) => e.key === 'Enter' && name.trim() && !busy && create()} />
          </div>
          <div className="field"><label>What kind of project is it?</label>
            {guess && <div className="small muted" style={{ marginBottom: 6 }}>Suggested <b>{CATEGORY_LABELS[guess.category]}</b> because the folder {guess.reason}.</div>}
            <div className="wl-types" role="radiogroup">
              {typeChoices(category).map(([k, v]) => (
                <button key={k} role="radio" aria-checked={category === k} className={cx('wl-type', category === k && 'on')} onClick={() => setCategory(k)}>
                  <ProjectIcon category={k} size={26} /><span>{v}</span>
                </button>
              ))}
            </div>
            <div className="small dim" style={{ marginTop: 6 }}>This decides how files are grouped and which files get earlier versions kept. You can change it later.</div>
          </div>
          <div className="row" style={{ justifyContent: 'flex-end' }}>
            <button className="btn primary" disabled={busy || !name.trim()} onClick={create}>{busy ? (mode === 'linked' ? 'Adding & indexing…' : 'Creating…') : mode === 'linked' ? 'Add project' : 'Create project'}</button>
          </div>
        </div>
      )}
    </div>
  );
}

/** Three pointers shown once, the first time a project is opened. */
export function FirstTips() {
  const [show, setShow] = useState(() => { try { return !localStorage.getItem('pv.tipsSeen'); } catch { return false; } });
  if (!show) return null;
  const done = () => { try { localStorage.setItem('pv.tipsSeen', '1'); } catch { /* ignore */ } setShow(false); };
  return (
    <div className="firsttips" role="note">
      <div className="ft-item"><LuFileText /><span><b>Click a file</b> to write what it’s for. The note follows the file if you rename or move it.</span></div>
      <div className="ft-item"><LuSearch /><span><b>Ctrl K</b> finds any project, file, note or function.</span></div>
      <div className="ft-item"><LuHistory /><span><b>Timeline</b> shows what changed — including edits you make in other programs.</span></div>
      <button className="btn ghost sm icon" onClick={done} title="Got it — don’t show again" aria-label="Dismiss tips"><LuX /></button>
    </div>
  );
}
