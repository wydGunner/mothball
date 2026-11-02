import { useEffect, useRef, useState } from 'react';
import { LuRefreshCw, LuSave, LuShieldCheck } from 'react-icons/lu';
import { call } from '../ipc';
import { Modal, TagInput } from '../components/common';

// When the Smart Save dialog appears (Settings → Editor & saving):
//   ask        — every Ctrl+S
//   ask-large  — only when at least N lines changed
//   auto       — never ask; Ctrl+S always backs up the on-disk version first
//   manual     — never ask; Ctrl+Shift+S / "Save with backup…" opens it
export type SaveMode = 'ask' | 'ask-large' | 'auto' | 'manual';
export const getSaveMode = (): SaveMode => (localStorage.getItem('pv.saveMode') as SaveMode) || 'ask';
export const setSaveMode = (m: SaveMode) => localStorage.setItem('pv.saveMode', m);
export const getLargeThreshold = () => Number(localStorage.getItem('pv.saveLargeLines')) || 20;
export const setLargeThreshold = (n: number) => localStorage.setItem('pv.saveLargeLines', String(Math.max(1, n)));

/** Rough count of changed lines (multiset difference) — only used to decide whether to ask. */
export function changedLines(a: string, b: string): number {
  const count = new Map<string, number>();
  for (const l of a.split('\n')) count.set(l, (count.get(l) || 0) + 1);
  let added = 0;
  for (const l of b.split('\n')) { const c = count.get(l) || 0; if (c > 0) count.set(l, c - 1); else added++; }
  let removed = 0;
  for (const c of count.values()) removed += c;
  return Math.max(added, removed);
}

export interface SmartSaveChoice {
  backupBefore: boolean;
  backupAfter: boolean;
  name: string;
  description: string;
  tags: string;
}

export function SmartSaveDialog({ fileId, fileName, projectName, projectId, relDir, linesChanged, onCancel, onSaveNormally, onSaveWithBackup }: {
  fileId: string; fileName: string; projectName: string; projectId: string; relDir: string; linesChanged: number;
  onCancel: () => void; onSaveNormally: () => void; onSaveWithBackup: (c: SmartSaveChoice) => void;
}) {
  const [description, setDescription] = useState('');
  const [name, setName] = useState('');
  const [nameEdited, setNameEdited] = useState(false);
  const [source, setSource] = useState('');
  const [tags, setTags] = useState<string[]>([]);
  const [taskTitle, setTaskTitle] = useState('');
  const [tasks, setTasks] = useState<{ id: string; title: string; kind: string }[]>([]);
  const [before, setBefore] = useState(true);
  const [after, setAfter] = useState(false);
  const [dontAsk, setDontAsk] = useState(false);
  const descRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => { descRef.current?.focus(); call<any[]>('tasks.list', { projectId }).then((t) => setTasks(t.filter((x) => x.status !== 'done'))).catch(() => {}); }, [projectId]);
  // offline suggestion from the description / task; neutral version+date when there's nothing to go on
  const suggest = (force = false) => {
    if (nameEdited && !force) return;
    call<{ name: string; source: string }>('versions.suggestName', { fileId, description, taskTitle }).then((r) => { setName(r.name); setSource(r.source); if (force) setNameEdited(false); }).catch(() => {});
  };
  useEffect(() => { const t = setTimeout(() => suggest(), 250); return () => clearTimeout(t); /* eslint-disable-next-line */ }, [description, taskTitle]);

  const finishNormal = () => { if (dontAsk) setSaveMode('manual'); onSaveNormally(); };
  const finishBackup = () => {
    if (dontAsk) setSaveMode('manual');
    const desc = taskTitle && !description.includes(taskTitle) ? `${description.trim()}${description.trim() ? '\n' : ''}Related task: ${taskTitle}` : description.trim();
    onSaveWithBackup({ backupBefore: before, backupAfter: after, name, description: desc, tags: tags.join(', ') });
  };

  return (
    <Modal wide title={<span className="row"><LuShieldCheck /> Save with backup</span>} onClose={onCancel} footer={<>
      <label className="row small" style={{ marginRight: 'auto' }}><input type="checkbox" checked={dontAsk} onChange={(e) => setDontAsk(e.target.checked)} /> Don't ask on every Ctrl+S</label>
      <button className="btn" onClick={onCancel}>Cancel</button>
      <button className="btn" onClick={finishNormal}><LuSave /> Save normally</button>
      <button className="btn primary" disabled={!before && !after} onClick={finishBackup}><LuShieldCheck /> Save with backup</button>
    </>}>
      <div className="ss-file">
        <div><span className="dim small">File</span><b className="mono">{fileName}</b></div>
        <div><span className="dim small">Project</span><span>{projectName}</span></div>
        <div><span className="dim small">Folder</span><span className="mono small">{relDir || '(project root)'}</span></div>
        <div><span className="dim small">Changes</span><span className="small">about {linesChanged} line{linesChanged === 1 ? '' : 's'} changed since it was opened or last saved</span></div>
      </div>

      <div className="field">
        <label>What changed in this version?</label>
        <textarea ref={descRef} className="input" rows={4} value={description} onChange={(e) => setDescription(e.target.value)}
          placeholder={'What you changed and why — e.g. “Instashoot fix: moved timing check before animation”.\nDid it work? Anything to watch out for? What should be tested next?'} />
      </div>
      <div className="grid2">
        <div className="field"><label>Backup name</label>
          <div className="row"><input className="input mono" value={name} onChange={(e) => { setName(e.target.value); setNameEdited(true); }} />
            <button className="btn icon" title="Suggest again from the description" onClick={() => suggest(true)}><LuRefreshCw /></button></div>
          <span className="small dim">{nameEdited ? 'Your name' : source === 'neutral' ? 'Neutral name (add a description for a descriptive one)' : source === 'task' ? 'Suggested from the related task' : 'Suggested from your description'}</span>
        </div>
        <div className="field"><label>Related task or bug (optional)</label>
          <select className="input" value={taskTitle} onChange={(e) => setTaskTitle(e.target.value)}>
            <option value="">None</option>{tasks.map((t) => <option key={t.id} value={t.title}>{t.kind === 'bug' ? 'Bug: ' : ''}{t.title}</option>)}
          </select>
        </div>
      </div>
      <div className="field"><label>Tags (optional)</label><TagInput value={tags} onChange={setTags} placeholder="e.g. working, needs-testing" /></div>

      <div className="ss-options">
        <label className="row"><input type="checkbox" checked={before} onChange={(e) => setBefore(e.target.checked)} />
          <div><b>Back up the file as it is on disk now, before saving</b><div className="small dim">Keeps the version from before your edits — named “{name ? name.replace(/(\.[^.]+)?$/, '_before$1') : '…_before'}”.</div></div></label>
        <label className="row"><input type="checkbox" checked={after} onChange={(e) => setAfter(e.target.checked)} />
          <div><b>Also keep a named copy of what you're saving now</b><div className="small dim">Keeps the new version as “{name || '…'}”, so you can always get back to exactly this.</div></div></label>
      </div>
    </Modal>
  );
}
