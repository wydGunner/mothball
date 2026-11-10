import { useEffect, useRef, useState } from 'react';
import { LuFolderOpen, LuHardDrive, LuTriangleAlert, LuShieldCheck, LuArchiveRestore, LuChevronDown, LuChevronRight } from 'react-icons/lu';
import { call, pickFolder, isTauri } from '../ipc';
import { Modal, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { ago, bytes, dateTime } from '../format';
import type { Nav } from '../main';

/** Polls the backup drive's status; faster while a job runs. */
function useDriveStatus() {
  const [s, setS] = useState<any>(null);
  const timer = useRef<any>(null);
  const load = async () => {
    try { setS(await call<any>('drive.status')); } catch { /* keep what's shown */ }
  };
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      if (!alive) return;
      let next: any = null;
      try { next = await call<any>('drive.status'); if (alive) setS(next); } catch { /* ignore */ }
      timer.current = setTimeout(tick, next?.running || next?.queued ? 700 : 5000);
    };
    tick();
    return () => { alive = false; clearTimeout(timer.current); };
  }, []);
  return { s, reload: load };
}

/** Waits for the background job queued at `since` to finish; returns its result. */
async function waitForJob(since: number): Promise<any> {
  for (;;) {
    await new Promise((r) => setTimeout(r, 600));
    const st = await call<any>('drive.status');
    if (st.lastJob && st.lastJob.finishedAt >= since && !st.running && !st.queued) return st.lastJob;
  }
}

function Running({ r }: { r: any }) {
  const pct = r.total > 0 ? Math.min(100, Math.round((r.done / r.total) * 100)) : null;
  return (
    <div style={{ margin: '10px 0' }}>
      <div className="row small"><span>{r.phase}…</span><span className="dim right">{pct !== null ? `${pct}%` : ''}{r.bytes ? ` · ${bytes(r.bytes)} copied` : ''}</span></div>
      <div className="progress" style={{ marginTop: 6 }}><div style={{ width: `${pct ?? 5}%` }} /></div>
    </div>
  );
}

const EVERY: [number, string][] = [[1, 'Every hour'], [6, 'Every 6 hours'], [24, 'Every day'], [72, 'Every 3 days'], [168, 'Every week']];

/** Warnings about a chosen folder: on the same drive as what it protects. */
function DestCheck({ c }: { c: any }) {
  if (!c) return null;
  if (!c.available) return <div className="alert warn small"><LuTriangleAlert /><div>That folder can't be reached. Connect the drive and try again.</div></div>;
  return (
    <>
      {c.sameDisk.length > 0 && <div className="alert warn small" style={{ marginBottom: 8 }}><LuTriangleAlert /><div>
        <b>This is the same drive{c.drive ? ` (${c.drive})` : ''} as {c.sameDisk.slice(0, 3).map((x: any) => x.what).join(', ')}{c.sameDisk.length > 3 ? ` and ${c.sameDisk.length - 3} more` : ''}.</b> If that drive fails, the backup goes with it. Use an external drive or a second internal drive.
      </div></div>}
      {c.existingRuns > 0 && <div className="alert info small" style={{ marginBottom: 8 }}><LuShieldCheck /><div>This drive already has {c.existingRuns} Mothball backup(s). New backups are added next to them and only copy what's new.</div></div>}
    </>
  );
}

export function BackupDriveCard({ nav }: { nav: Nav }) {
  const { s, reload } = useDriveStatus();
  const [dest, setDest] = useState('');
  const [chk, setChk] = useState<any>(null);
  const [more, setMore] = useState(false);
  const [showRuns, setShowRuns] = useState(false);
  const act = useAction();
  const toast = useToast();
  const confirm = useConfirm();
  useEffect(() => { if (s && !dest && s.dest) setDest(s.dest); }, [s, dest]);
  if (!s) return null;

  const choose = async () => {
    const p = await pickFolder('Folder on the backup drive');
    if (!p) return;
    setDest(p);
    setChk(await act(() => call('drive.check', { dest: p })));
  };
  const turnOn = async () => {
    const c = chk || (await act(() => call<any>('drive.check', { dest })));
    if (!c) return;
    if (c.sameDisk.length > 0 && !(await confirm({ title: 'Back up to the same drive?', confirm: 'Use it anyway', danger: true,
      body: <>This folder is on the same drive as what it's meant to protect. It still guards against deleting or overwriting something by mistake, but not against the drive failing.</> }))) return;
    const r = await act(() => call('drive.setSettings', { settings: { enabled: true, dest } }));
    if (r) {
      setChk(null);
      await act(() => call('drive.runNow'));
      toast('Backup drive on — the first backup has started', 'success');
      reload();
    }
  };
  const save = async (patch: any) => {
    const r = await act(() => call('drive.setSettings', { settings: patch }), 'Saved');
    if (r) reload();
  };
  const run = async (cmd: 'drive.runNow' | 'drive.verify') => { if (await act(() => call(cmd))) reload(); };

  const last = s.lastResult;
  const running = s.running;
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2 className="row"><LuHardDrive /> Backup drive</h2>
      {!s.enabled ? <>
        <p className="muted small" style={{ marginTop: 0 }}>
          Keeps a complete backup on another drive: your project files, every saved version, snapshot and checkpoint, and Mothball's own data.
          After the first one, only what changed is copied. It runs in the background, and if the drive isn't plugged in it waits and catches up as soon as it is.
        </p>
        <div className="row"><input className="input mono" value={dest} onChange={(e) => { setDest(e.target.value); setChk(null); }} onBlur={async () => dest && setChk(await call('drive.check', { dest }).catch(() => null))} placeholder="E:\" />
          <button className="btn" onClick={choose}><LuFolderOpen /> Choose…</button>
          <button className="btn primary" disabled={!dest} onClick={turnOn}>Turn on</button></div>
        <div style={{ marginTop: 8 }}><DestCheck c={chk} /></div>
      </> : <>
        {running ? <Running r={running} />
          : !s.available ? <div className="alert warn small"><LuTriangleAlert /><div className="grow"><b>Waiting for the drive.</b> <span className="mono">{s.dest}</span> isn't connected. It'll back up as soon as you plug it in{s.lastOk ? ` (last backup ${ago(s.lastOk)})` : ''}.</div></div>
          : last && !last.ok ? <div className="alert bad small"><LuTriangleAlert /><div className="grow"><b>The last backup didn't finish.</b> {last.error}</div></div>
          : null}
        {!running && <div className="kv small" style={{ margin: '6px 0 10px' }}>
          <span>Drive</span><span className="mono">{s.dest}</span>
          <span>Last backup</span><span>{s.lastOk ? <>{ago(s.lastOk)} <span className="dim">({dateTime(s.lastOk)})</span></> : 'Not yet'}</span>
          {last?.ok && <><span>Contains</span><span>{last.files.toLocaleString()} project files and {last.historyItems.toLocaleString()} saved versions{last.newFiles + last.newHistoryItems > 0 ? <span className="dim"> · last run copied {(last.newFiles + last.newHistoryItems).toLocaleString()} new ({bytes(last.bytesCopied)})</span> : <span className="dim"> · nothing new last time</span>}</span></>}
          {last?.ok && last.carriedOver?.length > 0 && <><span>Not reachable</span><span>{last.carriedOver.join(', ')} — kept from the previous backup</span></>}
          {last?.ok && last.skippedCount > 0 && <><span>Skipped</span><span title={last.skipped.map((x: any) => `${x.project}/${x.path}: ${x.reason}`).join('\n')}>{last.skippedCount} file(s) — {last.skipped[0]?.reason}{last.skippedCount > 1 ? '…' : ''}</span></>}
          {s.available && s.nextRun && <><span>Next</span><span>{s.nextRun <= Date.now() ? 'Soon' : dateTime(s.nextRun)}</span></>}
          {s.lastVerify && <><span>Last full check</span><span>{ago(s.lastVerify.at)} — {s.lastVerify.ok ? `all ${s.lastVerify.checked.toLocaleString()} files OK` : <span style={{ color: 'var(--bad)' }}>{s.lastVerify.damaged} damaged file(s) removed; the next backup copies them again</span>}</span></>}
        </div>}
        <div className="row">
          <button className="btn primary" disabled={!!running || !s.available} onClick={() => run('drive.runNow')}>Back up now</button>
          <button className="btn" disabled={!!running || !s.available} title="Read back every file in the newest backup and check it — slow on big backups" onClick={() => run('drive.verify')}>Check every file</button>
          <button className="btn ghost right" onClick={() => setMore((m) => !m)}>{more ? <LuChevronDown /> : <LuChevronRight />} Settings</button>
        </div>
        {more && <div style={{ marginTop: 12 }}>
          <div className="grid2">
            <div className="field"><label>How often</label>
              <select className="input" value={s.everyHours} onChange={(e) => save({ everyHours: Number(e.target.value) })}>
                {EVERY.map(([h, l]) => <option key={h} value={h}>{l}</option>)}
              </select></div>
            <div className="field"><label>Keep</label>
              <select className="input" title="Older backups are removed; files only they had are freed" value={s.keep} onChange={(e) => save({ keep: Number(e.target.value) })}>
                {[7, 14, 30, 60, 120].map((n) => <option key={n} value={n}>Newest {n} backups</option>)}
              </select></div>
          </div>
          <div className="field"><label>Folder</label>
            <div className="row"><input className="input mono" value={dest} onChange={(e) => setDest(e.target.value)} />
              <button className="btn" onClick={choose}><LuFolderOpen /> Choose…</button>
              <button className="btn" disabled={dest === s.dest} onClick={async () => {
                const c = chk || (await act(() => call<any>('drive.check', { dest })));
                if (c?.sameDisk?.length && !(await confirm({ title: 'Back up to the same drive?', confirm: 'Use it anyway', danger: true, body: 'This folder is on the same drive as what it protects, so a drive failure would take the backup too.' }))) return;
                save({ dest });
              }}>Use this folder</button></div>
            <DestCheck c={chk} />
          </div>
          <button className="btn ghost sm" onClick={async () => { if (await confirm({ title: 'Turn off the backup drive?', confirm: 'Turn off', body: 'No new backups will be made. Backups already on the drive stay there.' })) save({ enabled: false }); }}>Turn off</button>
        </div>}
        {s.available && <div style={{ marginTop: 12 }}>
          <button className="linklike small" onClick={() => setShowRuns((x) => !x)}>{showRuns ? <LuChevronDown /> : <LuChevronRight />} Backups on this drive</button>
          {showRuns && <RunList nav={nav} />}
        </div>}
      </>}
    </div>
  );
}

/** Backups on a drive; `dest` for a drive that isn't the configured one. */
function RunList({ nav, dest }: { nav: Nav; dest?: string }) {
  const runs = useLoad(() => call<any>('drive.runs', { dest: dest || '' }), [dest]);
  const [open, setOpen] = useState<any>(null);
  if (runs.error) return <div className="alert bad small" style={{ marginTop: 8 }}>{runs.error}</div>;
  if (!runs.data) return <div className="small dim" style={{ marginTop: 8 }}>Reading the drive…</div>;
  if (!runs.data.runs.length) return <div className="small dim" style={{ marginTop: 8 }}>No backups on this drive yet.</div>;
  return (
    <>
      <table className="list" style={{ marginTop: 8 }}><tbody>{runs.data.runs.map((r: any) => (
        <tr key={r.id}>
          <td>{dateTime(r.createdAt)} <span className="dim small">{ago(r.createdAt)}</span>{!r.mine && <span className="badge" style={{ marginLeft: 6 }} title="Made by Mothball on another PC, or before a reinstall">other PC</span>}</td>
          <td className="small dim">{r.projects.length} project(s) · {r.files.toLocaleString()} files · {bytes(r.bytes)}</td>
          <td style={{ textAlign: 'right' }}><button className="btn sm" onClick={() => setOpen(r)}><LuArchiveRestore /> Restore…</button></td>
        </tr>))}</tbody></table>
      {open && <RestoreDialog nav={nav} run={open} dest={dest} onClose={() => setOpen(null)} />}
    </>
  );
}

function RestoreDialog({ nav, run, dest, onClose }: { nav: Nav; run: any; dest?: string; onClose: () => void }) {
  const here = new Set(nav.projects.map((p) => p.id));
  const [pid, setPid] = useState<string>(() => (run.projects.find((p: any) => here.has(p.id)) || run.projects[0])?.id || '');
  const pv = useLoad(() => (pid ? call<any>('drive.restorePreview', { dest: dest || '', runId: run.id, projectId: pid }) : Promise.resolve(null)), [pid]);
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [res, setRes] = useState<any>(null);
  const act = useAction();
  const confirm = useConfirm();
  const n = pv.data ? pv.data.recreate + (overwrite ? pv.data.differs : 0) : 0;
  const start = async (cmd: string, args: any, label: string) => {
    const since = Date.now();
    if (!(await act(() => call(cmd, { dest: dest || '', runId: run.id, ...args })))) return;
    setBusy(label);
    const r = await waitForJob(since);
    setBusy(null);
    setRes(r);
    if (r.ok) nav.refreshProjects();
  };
  const all = async () => {
    if (!(await confirm({ title: 'Restore everything from this backup?', confirm: 'Restore everything', danger: true,
      body: <>For a new PC, or after losing the drive Mothball was on. Every project, note, task, version and setting goes back to how it was on <b>{dateTime(run.createdAt)}</b>.
        Project files that are missing are recreated; files that exist are left alone. The current database is kept in the data folder's <span className="mono">backups</span> folder, and the switch happens when Mothball restarts.</> }))) return;
    start('drive.restoreAll', {}, 'Restoring everything');
  };
  if (res) return (
    <Modal title={res.ok ? 'Restored' : 'Restored, with problems'} onClose={onClose} footer={<>
      {res.kind === 'restoreAll' && isTauri && <button className="btn primary" onClick={async () => { const { relaunch } = await import('@tauri-apps/plugin-process'); await relaunch(); }}>Restart now</button>}
      <button className={res.kind === 'restoreAll' && isTauri ? 'btn' : 'btn primary'} onClick={onClose}>Done</button></>}>
      {res.error ? <div className="alert bad">{res.error}</div>
        : res.kind === 'restoreAll' ? <p>{res.restoredFiles.toLocaleString()} project file(s) and the file history were restored. <b>Restart Mothball</b> to switch to the restored projects, notes and settings.</p>
        : <p>{res.restored} file(s) restored into <span className="mono">{res.root}</span> and checked.{res.keptDifferent ? ` ${res.keptDifferent} file(s) that differ were left as they are.` : ''}</p>}
      {res.errors?.length > 0 && <div className="alert bad"><div><b>{res.errors.length} problem(s):</b><ul className="small">{res.errors.slice(0, 20).map((e: string) => <li key={e}>{e}</li>)}</ul></div></div>}
    </Modal>
  );
  return (
    <Modal wide title={`Backup from ${dateTime(run.createdAt)}`} onClose={busy ? () => {} : onClose} footer={busy ? <span className="small dim">{busy}… this can take a while for big projects.</span> : <>
      <button className="btn ghost" style={{ marginRight: 'auto' }} onClick={all}>Restore everything…</button>
      <button className="btn" onClick={onClose}>Cancel</button>
      <button className="btn primary" disabled={!pv.data || !n} onClick={() => start('drive.restoreProject', { projectId: pid, overwrite }, 'Restoring files')}>Restore {n.toLocaleString()} file(s)</button></>}>
      <div className="field"><label>Project</label>
        <select className="input" value={pid} onChange={(e) => { setPid(e.target.value); setOverwrite(false); }}>
          {run.projects.map((p: any) => <option key={p.id} value={p.id}>{p.name} ({p.files} files){here.has(p.id) ? '' : ' — not in Mothball now'}</option>)}
        </select></div>
      {pv.error ? <div className="alert bad">{pv.error}</div> : !pv.data ? <div className="muted small">Comparing with the files on disk…</div> : <>
        <p style={{ marginTop: 0 }}>Into <span className="mono" style={{ wordBreak: 'break-all' }}>{pv.data.root}</span>{!pv.data.rootExists && ' (this folder will be created)'}:</p>
        <ul className="small">
          <li><b>{pv.data.recreate}</b> missing file(s) will be recreated</li>
          <li><b>{pv.data.differs}</b> file(s) differ from the backup — {overwrite ? <b>they'll be replaced</b> : 'left as they are'}</li>
          <li><b>{pv.data.same}</b> already match</li>
          <li>Files that aren't in the backup are never touched or deleted</li>
        </ul>
        {pv.data.differs > 0 && <label className="row"><input type="checkbox" checked={overwrite} onChange={(e) => setOverwrite(e.target.checked)} /> Also replace the {pv.data.differs} file(s) that differ (their current content is kept in each file's history first)</label>}
        {pv.data.items.length > 0 && <details><summary className="small">Show files</summary><ul className="small mono">{pv.data.items.slice(0, 300).map((x: any) => <li key={x.path}>{x.state} · {x.path}</li>)}</ul></details>}
      </>}
    </Modal>
  );
}

/** Restore section: pick any drive (e.g. on a new PC, before setting one up). */
export function DriveRestoreCard({ nav }: { nav: Nav }) {
  const [dest, setDest] = useState('');
  const [shown, setShown] = useState('');
  return (
    <div className="card pad" style={{ marginBottom: 16 }}>
      <h2 className="row"><LuHardDrive /> From a backup drive</h2>
      <p className="muted small" style={{ marginTop: 0 }}>Bring back one project's files, or everything (on a new PC, say). Choose the drive, or its <span className="mono">Mothball Backup</span> folder.</p>
      <div className="row"><input className="input mono" value={dest} onChange={(e) => setDest(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && setShown(dest)} placeholder="E:\" />
        <button className="btn" onClick={async () => { const p = await pickFolder('Backup drive'); if (p) { setDest(p); setShown(p); } }}><LuFolderOpen /> Choose…</button>
        <button className="btn primary" disabled={!dest} onClick={() => setShown(dest)}>Show backups</button></div>
      {shown && <RunList nav={nav} dest={shown} key={shown} />}
    </div>
  );
}

/** Top-bar chip when backups to the drive have stopped happening. */
export function DriveWarning({ onOpen }: { onOpen: () => void }) {
  const [s, setS] = useState<any>(null);
  useEffect(() => {
    let alive = true;
    const tick = () => call<any>('drive.status').then((x) => alive && setS(x)).catch(() => {});
    tick();
    const t = setInterval(tick, 60_000);
    return () => { alive = false; clearInterval(t); };
  }, []);
  if (!s?.enabled || s.running) return null;
  const failed = s.available && s.lastResult && !s.lastResult.ok;
  if (!s.overdue && !failed) return null;
  const days = s.lastOk ? Math.floor((Date.now() - s.lastOk) / 86_400_000) : 0;
  const msg = failed ? 'Backup failed' : s.lastOk ? `No backup for ${days} day${days === 1 ? '' : 's'}` : 'Backup drive not connected';
  return (
    <button className="btn sm drive-warn" title={failed ? s.lastResult.error : `Plug in ${s.dest} so Mothball can back up`} onClick={onOpen}><LuTriangleAlert /> {msg}</button>
  );
}
