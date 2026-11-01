import { useEffect, useState } from 'react';
import { LuTriangleAlert, LuGitCompareArrows, LuPlus, LuTrash2, LuCircleCheck, LuCircleX, LuFileWarning, LuCopy } from 'react-icons/lu';
import { call } from '../ipc';
import { TagInput, useAction, useLoad, useConfirm, useToast, KindIcon } from '../components/common';
import { ago, bytes, CATEGORY_LABELS, dateTime } from '../format';
import type { ProjectCtx } from './Project';

type FieldDef = { key: string; label: string; multiline?: boolean; options?: string[]; placeholder?: string };
const MODULES: Record<string, { title: string; fields: FieldDef[] }> = {
  software: { title: 'Software details', fields: [
    { key: 'repoUrl', label: 'Repository URL' }, { key: 'language', label: 'Languages / runtime' },
    { key: 'buildInstructions', label: 'Build instructions', multiline: true }, { key: 'dependencies', label: 'Dependencies', multiline: true },
    { key: 'releaseHistory', label: 'Release history', multiline: true, placeholder: 'v1.2 — 2026-08-01 — …' },
  ] },
  game: { title: 'Game development details', fields: [
    { key: 'engine', label: 'Engine', placeholder: 'Unity' }, { key: 'engineVersion', label: 'Engine version', placeholder: '6000.0.x' },
    { key: 'platforms', label: 'Target platforms' }, { key: 'renderPipeline', label: 'Render pipeline' },
    { key: 'sceneNotes', label: 'Scenes & key scripts', multiline: true }, { key: 'buildNotes', label: 'Build notes', multiline: true },
    { key: 'playtestNotes', label: 'Playtesting notes', multiline: true },
  ] },
  '3d-printing': { title: '3D printing details', fields: [
    { key: 'printStatus', label: 'Print status', options: ['', 'IDEA', 'MODELED', 'SOURCE_ONLY', 'SLICED_UNTESTED', 'TEST_FIT_PASSED', 'REPRINT_READY', 'NEEDS_WORK', 'FAILED_PRINT', 'ARCHIVED'] },
    { key: 'material', label: 'Material', placeholder: 'PLA' }, { key: 'filamentColor', label: 'Filament color(s)' },
    { key: 'filamentGrams', label: 'Estimated filament (g)' }, { key: 'printTime', label: 'Print duration', placeholder: '5h 30m' },
    { key: 'printer', label: 'Printer' }, { key: 'nozzle', label: 'Nozzle' }, { key: 'layerHeight', label: 'Layer height' },
    { key: 'slicer', label: 'Slicer' }, { key: 'slicerProfile', label: 'Slicer profile' }, { key: 'dimensions', label: 'Dimensions (mm)' },
    { key: 'fitsContains', label: 'Fits / contains' },
    { key: 'assemblyNotes', label: 'Assembly notes', multiline: true }, { key: 'finishingNotes', label: 'Finishing instructions', multiline: true },
  ] },
  electronics: { title: 'Electronics details', fields: [
    { key: 'board', label: 'Board / PCB' }, { key: 'mcu', label: 'MCU' }, { key: 'firmware', label: 'Firmware' },
    { key: 'bom', label: 'Bill of materials', multiline: true }, { key: 'wiringNotes', label: 'Wiring / pinout notes', multiline: true },
  ] },
};

export function Overview({ ctx }: { ctx: ProjectCtx }) {
  const p = ctx.project;
  const act = useAction();
  const [form, setForm] = useState({ name: p.name, description: p.description, icon: p.icon, ignoreGlobs: p.ignoreGlobs });
  const [module, setModule] = useState<Record<string, any>>(p.module || {});
  const [dirty, setDirty] = useState(false);
  useEffect(() => { setForm({ name: p.name, description: p.description, icon: p.icon, ignoreGlobs: p.ignoreGlobs }); setModule(p.module || {}); setDirty(false); }, [p.id]);
  const statuses = useLoad(() => call<{ name: string }[]>('projects.statuses'), []);
  const reviews = useLoad(() => call<any[]>('files.reviews', { projectId: p.id }), [p.id, ctx.scanTick]);
  const activity = useLoad(() => call<any[]>('files.activity', { projectId: p.id, limit: 12 }), [p.id, ctx.scanTick]);
  const changed = useLoad(() => call<any[]>('files.list', { projectId: p.id, changedOnly: true, limit: 50 }), [p.id, ctx.scanTick]);
  const missing = useLoad(() => call<any[]>('files.list', { projectId: p.id, state: 'missing', limit: 50 }), [p.id, ctx.scanTick]);
  const tasks = useLoad(() => call<any[]>('tasks.list', { projectId: p.id }), [p.id]);
  const confirm = useConfirm();
  const toast = useToast();
  const [statusText, setStatusText] = useState('');

  const patch = async (pt: Record<string, unknown>) => { await act(() => call('projects.update', { id: p.id, patch: pt })); await ctx.reload(); ctx.nav.refreshProjects(); };
  const saveForm = async () => { await patch({ ...form, module }); setDirty(false); toast('Saved', 'success'); };
  const mod = MODULES[p.category];
  const nextTasks = (tasks.data || []).filter((t) => t.status !== 'done').slice(0, 6);

  return (
    <div className="page" style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1.4fr) minmax(320px, 1fr)', gap: 18, alignItems: 'start' }}>
      <div className="col" style={{ gap: 16 }}>
        <div className="card pad">
          <div className="statrow">
            <div className="stat"><b>{p.fileCount}</b><span>files</span></div>
            <div className="stat"><b>{bytes(p.bytes)}</b><span>storage</span></div>
            <div className="stat"><b>{p.openTasks}</b><span>open tasks</span></div>
            <div className="stat"><b style={{ color: p.changedFiles ? 'var(--warn)' : undefined }}>{p.changedFiles}</b><span>changed since reviewed</span></div>
            <div className="stat"><b style={{ color: p.missingFiles ? 'var(--bad)' : undefined }}>{p.missingFiles}</b><span>missing</span></div>
          </div>
        </div>

        {(reviews.data?.length || 0) > 0 && (
          <div className="card pad">
            <h2 className="row"><LuGitCompareArrows color="var(--warn)" /> Confirm file identity <span className="badge warn">{reviews.data!.length}</span></h2>
            <p className="muted small">These files disappeared while a similar file appeared elsewhere, but the match wasn't certain enough to move documentation automatically. Confirm only if the new file really is the old one.</p>
            {reviews.data!.map((r) => (
              <div key={r.id} className="row" style={{ padding: '8px 0', borderTop: '1px solid var(--line)' }}>
                <div className="grow small">
                  <div><span className="dim">was</span> <span className="mono">{r.missing.path}</span> {r.missing.hasDocs && <span className="tag">has notes</span>}</div>
                  <div><span className="dim">now?</span> <span className="mono">{r.candidate.path}</span> <span className="dim">· {r.reason}</span></div>
                </div>
                <button className="btn sm" onClick={async () => { await act(() => call('files.resolveReview', { id: r.id, accept: true }), 'Linked — notes and history now follow the file'); reviews.reload(); ctx.reload(); }}><LuCircleCheck /> Same file</button>
                <button className="btn sm" onClick={async () => { await act(() => call('files.resolveReview', { id: r.id, accept: false })); reviews.reload(); ctx.reload(); }}><LuCircleX /> Different</button>
              </div>
            ))}
          </div>
        )}

        {(changed.data?.length || 0) > 0 && (
          <div className="card pad">
            <h2 className="row"><LuFileWarning color="var(--warn)" /> Changed outside Project Vault
              <button className="btn sm right" onClick={async () => { await act(() => call('files.markReviewed', { ids: changed.data!.map((f) => f.id) })); changed.reload(); ctx.reload(); }}>Mark all reviewed</button></h2>
            {changed.data!.map((f) => (
              <div key={f.id} className="row small" style={{ padding: '4px 0', cursor: 'pointer' }} onClick={() => ctx.openFile(f.id)}>
                <KindIcon kind={f.kind} /><span className="mono grow">{f.path}</span>{f.hasDocs && <span className="tag">documented</span>}<span className="dim">{ago(f.lastChanged)}</span>
              </div>
            ))}
          </div>
        )}

        {(missing.data?.length || 0) > 0 && (
          <div className="card pad">
            <h2 className="row"><LuTriangleAlert color="var(--bad)" /> Missing files</h2>
            <p className="muted small">No longer found on disk. Their documentation is kept; if a file comes back (or is restored from Git/backup) it is re-attached automatically.</p>
            {missing.data!.map((f) => (
              <div key={f.id} className="row small" style={{ padding: '3px 0' }}>
                <KindIcon kind={f.kind} /><span className="mono grow" style={{ cursor: 'pointer' }} onClick={() => ctx.openFile(f.id)}>{f.path}</span>{f.hasDocs && <span className="tag">documented</span>}
                <button className="btn ghost sm" onClick={async () => {
                  if (await confirm({ title: 'Forget this file?', danger: true, confirm: 'Forget', body: <>The record for <span className="mono">{f.path}</span>{f.hasDocs ? ', including its documentation,' : ''} is deleted from the vault. Nothing on disk changes.</> })) {
                    await act(() => call('files.forgetMissing', { id: f.id })); missing.reload(); ctx.reload();
                  }
                }}>Forget</button>
              </div>
            ))}
          </div>
        )}

        <div className="card pad">
          <h2 style={{ marginBottom: 12 }}>Project</h2>
          <div className="grid2">
            <div className="field"><label>Name</label><input className="input" value={form.name} onChange={(e) => { setForm({ ...form, name: e.target.value }); setDirty(true); }} /></div>
            <div className="field"><label>Icon (emoji, optional)</label><input className="input" maxLength={4} value={form.icon} onChange={(e) => { setForm({ ...form, icon: e.target.value }); setDirty(true); }} placeholder="🛠" /></div>
          </div>
          <div className="field"><label>Description</label><textarea className="input" rows={3} value={form.description} onChange={(e) => { setForm({ ...form, description: e.target.value }); setDirty(true); }} /></div>
          <div className="grid3">
            <div className="field"><label>Status</label>
              <select className="input" value={p.status} onChange={(e) => patch({ status: e.target.value })}>{(statuses.data || []).map((s) => <option key={s.name}>{s.name}</option>)}</select>
            </div>
            <div className="field"><label>Custom status</label>
              <div className="row"><input className="input" value={statusText} onChange={(e) => setStatusText(e.target.value)} placeholder="e.g. Waiting on parts" /><button className="btn" disabled={!statusText.trim()} onClick={() => { patch({ status: statusText.trim() }); setStatusText(''); }}>Set</button></div>
            </div>
            <div className="field"><label>Category</label>
              <select className="input" value={p.category} onChange={(e) => patch({ category: e.target.value })}>{Object.entries(CATEGORY_LABELS).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
            </div>
          </div>
          <div className="field"><label>Progress — {p.progress}%</label><input type="range" min={0} max={100} step={5} defaultValue={p.progress} key={p.progress} onMouseUp={(e) => patch({ progress: Number((e.target as HTMLInputElement).value) })} onKeyUp={(e) => patch({ progress: Number((e.target as HTMLInputElement).value) })} /></div>
          <div className="field"><label>Tags</label><TagInput value={p.tags} onChange={(tags) => patch({ tags })} /></div>
          {mod && <>
            <h2 style={{ margin: '18px 0 12px' }}>{mod.title}</h2>
            <div className="grid2">
              {mod.fields.filter((f) => !f.multiline).map((f) => (
                <div className="field" key={f.key}><label>{f.label}</label>
                  {f.options
                    ? <select className="input" value={module[f.key] || ''} onChange={(e) => { setModule({ ...module, [f.key]: e.target.value }); setDirty(true); }}>{f.options.map((o) => <option key={o} value={o}>{o || '—'}</option>)}</select>
                    : <input className="input" value={module[f.key] || ''} placeholder={f.placeholder} onChange={(e) => { setModule({ ...module, [f.key]: e.target.value }); setDirty(true); }} />}
                </div>
              ))}
            </div>
            {mod.fields.filter((f) => f.multiline).map((f) => (
              <div className="field" key={f.key}><label>{f.label}</label><textarea className="input" rows={3} value={module[f.key] || ''} placeholder={f.placeholder} onChange={(e) => { setModule({ ...module, [f.key]: e.target.value }); setDirty(true); }} /></div>
            ))}
          </>}
          {p.category === '3d-printing' && <PrintLog log={module.printLog || []} onChange={(printLog) => { setModule({ ...module, printLog }); setDirty(true); }} />}
          <CustomFields value={module.custom || {}} onChange={(custom) => { setModule({ ...module, custom }); setDirty(true); }} />
          <details style={{ marginTop: 10 }}>
            <summary className="muted" style={{ cursor: 'pointer' }}>Indexing rules (folders and file types to skip)</summary>
            <p className="small dim">One rule per line: <code>Library/</code> (folder anywhere), <code>Assets/Generated/</code> (folder path), <code>*.tmp</code>, or a file name. .git, node_modules and system files are always skipped.</p>
            <textarea className="input code" rows={5} value={form.ignoreGlobs} onChange={(e) => { setForm({ ...form, ignoreGlobs: e.target.value }); setDirty(true); }} />
          </details>
          <div className="row" style={{ marginTop: 14 }}>
            <button className="btn primary" disabled={!dirty} onClick={saveForm}>Save changes</button>
            {dirty && <span className="small" style={{ color: 'var(--warn)' }}>Unsaved changes</span>}
          </div>
        </div>
      </div>

      <div className="col" style={{ gap: 16 }}>
        <div className="card pad">
          <h2 style={{ marginBottom: 10 }}>What's next</h2>
          {nextTasks.length ? nextTasks.map((t) => (
            <div key={t.id} className="row small" style={{ padding: '4px 0', cursor: 'pointer' }} onClick={() => ctx.openTask(t.id)}>
              <span className={`prio p${t.priority}`}>{['low', 'normal', 'high', 'urgent'][t.priority]}</span>
              <span className="grow">{t.kind === 'bug' ? '🐞 ' : ''}{t.title}</span><span className="dim">{t.status.replace('_', ' ')}</span>
            </div>
          )) : <div className="muted small">No open tasks.</div>}
          <button className="btn sm" style={{ marginTop: 8 }} onClick={() => ctx.openTask('')}>Open task board</button>
        </div>
        <div className="card pad">
          <h2 style={{ marginBottom: 10 }}>Recent file activity</h2>
          {(activity.data || []).length ? activity.data!.map((a, i) => (
            <div key={i} className="row small" style={{ padding: '3px 0', cursor: 'pointer' }} onClick={() => ctx.openFile(a.fileId)}>
              <span className="pill" style={{ minWidth: 70, justifyContent: 'center' }}>{a.kind}</span>
              <span className="mono grow" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={a.detail || a.path}>{a.kind === 'moved' ? a.detail : a.path}</span>
              <span className="dim">{ago(a.at)}</span>
            </div>
          )) : <div className="muted small">No changes detected since the project was indexed.</div>}
        </div>
        <div className="card pad">
          <h2 style={{ marginBottom: 10 }}>Storage</h2>
          <div className="kv">
            <span>Mode</span><span>{p.mode === 'linked' ? 'Linked folder (files stay where they are)' : 'Managed by Project Vault'}</span>
            <span>Location</span><span className="mono row">{p.rootPath}<button className="btn ghost sm icon" title="Copy path" onClick={() => { navigator.clipboard.writeText(p.rootPath); toast('Path copied'); }}><LuCopy /></button></span>
            <span>Created</span><span>{dateTime(p.createdAt)}</span>
            <span>Last scan</span><span>{dateTime(p.lastScanAt)}</span>
          </div>
        </div>
      </div>
    </div>
  );
}

function PrintLog({ log, onChange }: { log: any[]; onChange: (l: any[]) => void }) {
  const [draft, setDraft] = useState({ date: new Date().toISOString().slice(0, 10), result: 'success', notes: '', grams: '', time: '' });
  return (
    <div style={{ marginTop: 6 }}>
      <h3 style={{ marginBottom: 8 }}>Print history</h3>
      {log.length === 0 && <div className="small dim" style={{ marginBottom: 8 }}>No prints logged yet.</div>}
      {log.map((e, i) => (
        <div key={i} className="row small" style={{ padding: '3px 0' }}>
          {e.result === 'success' ? <LuCircleCheck color="var(--ok)" /> : <LuCircleX color="var(--bad)" />}
          <span style={{ width: 90 }}>{e.date}</span><span className="grow">{e.notes}</span><span className="dim">{[e.grams && `${e.grams} g`, e.time].filter(Boolean).join(' · ')}</span>
          <button className="btn ghost sm icon" title="Remove" onClick={() => onChange(log.filter((_, j) => j !== i))}><LuTrash2 /></button>
        </div>
      ))}
      <div className="row" style={{ marginTop: 6 }}>
        <input className="input" type="date" style={{ width: 140 }} value={draft.date} onChange={(e) => setDraft({ ...draft, date: e.target.value })} />
        <select className="input" style={{ width: 110 }} value={draft.result} onChange={(e) => setDraft({ ...draft, result: e.target.value })}><option value="success">Success</option><option value="failure">Failure</option></select>
        <input className="input" placeholder="Notes (settings, what went wrong…)" value={draft.notes} onChange={(e) => setDraft({ ...draft, notes: e.target.value })} />
        <input className="input" style={{ width: 70 }} placeholder="g" value={draft.grams} onChange={(e) => setDraft({ ...draft, grams: e.target.value })} />
        <input className="input" style={{ width: 80 }} placeholder="time" value={draft.time} onChange={(e) => setDraft({ ...draft, time: e.target.value })} />
        <button className="btn" onClick={() => { onChange([...log, draft]); setDraft({ ...draft, notes: '', grams: '', time: '' }); }}><LuPlus /> Log</button>
      </div>
    </div>
  );
}

export function CustomFields({ value, onChange }: { value: Record<string, string>; onChange: (v: Record<string, string>) => void }) {
  const [k, setK] = useState('');
  const [v, setV] = useState('');
  return (
    <div style={{ marginTop: 14 }}>
      <h3 style={{ marginBottom: 8 }}>Custom fields</h3>
      {Object.entries(value).map(([key, val]) => (
        <div key={key} className="row" style={{ marginBottom: 6 }}>
          <span className="small" style={{ width: 140, color: 'var(--fg2)' }}>{key}</span>
          <input className="input" value={val} onChange={(e) => onChange({ ...value, [key]: e.target.value })} />
          <button className="btn ghost sm icon" title="Remove" onClick={() => { const n = { ...value }; delete n[key]; onChange(n); }}><LuTrash2 /></button>
        </div>
      ))}
      <div className="row">
        <input className="input" style={{ width: 140 }} placeholder="Field" value={k} onChange={(e) => setK(e.target.value)} />
        <input className="input" placeholder="Value" value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter' && k.trim()) { onChange({ ...value, [k.trim()]: v }); setK(''); setV(''); } }} />
        <button className="btn" disabled={!k.trim()} onClick={() => { onChange({ ...value, [k.trim()]: v }); setK(''); setV(''); }}><LuPlus /> Add</button>
      </div>
    </div>
  );
}
