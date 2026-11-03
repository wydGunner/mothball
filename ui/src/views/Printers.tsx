// Read-only Bambu Lab printer status: settings, live status, print history.
import { useEffect, useState } from 'react';
import { LuPrinter, LuCircleCheck, LuCircleX, LuCircleSlash, LuTrash2, LuPencil, LuPlus, LuShieldCheck } from 'react-icons/lu';
import { call, type Project } from '../ipc';
import { useAction, useConfirm } from '../components/common';
import { cx } from '../format';
import type { Nav } from '../main';

export interface PrinterStatus {
  id: string; name: string; host: string; serial: string; hasCode: boolean; certPinned: boolean;
  state: 'connecting' | 'online' | 'offline' | 'error'; message?: string; updatedAt?: number;
  stage?: string; job?: string; progress?: number; remainingMin?: number; layer?: number; totalLayers?: number;
  nozzle?: number; bed?: number; error?: string; hms?: number;
}
export interface PrintRecord {
  key: string; printerId: string; printer: string; name: string; file: string; result: 'success' | 'failure' | 'stopped';
  error?: string; startedAt?: number; endedAt: number; minutes?: number; projectId?: string; projectName?: string;
}

const ACTIVE = new Set(['prepare', 'running', 'pause', 'slicing']);
export const fmtMin = (m: number) => (m >= 60 ? `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m` : `${m}m`);
const when = (ms: number) => new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

/** Live printer list, refreshed every few seconds while there are printers. */
export function usePrinters(): PrinterStatus[] | null {
  const [list, setList] = useState<PrinterStatus[] | null>(null);
  useEffect(() => {
    let off = false;
    let t: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const l = await call<PrinterStatus[]>('printers.list');
        if (off) return;
        setList(l);
        t = setTimeout(tick, l.length ? 3000 : 15000);
      } catch { if (!off) t = setTimeout(tick, 15000); }
    };
    tick();
    return () => { off = true; if (t) clearTimeout(t); };
  }, []);
  return list;
}

export function usePrintHistory(stamp: unknown): [PrintRecord[], () => void] {
  const [h, setH] = useState<PrintRecord[]>([]);
  const [n, setN] = useState(0);
  useEffect(() => { call<PrintRecord[]>('printers.history', { limit: 30 }).then(setH).catch(() => {}); }, [stamp, n]);
  return [h, () => setN((x) => x + 1)];
}

/** One-line summary for a printer, e.g. "Printing Desk Cat · 45% · 1h 12m left". */
export function printerLine(p: PrinterStatus): string {
  if (p.state === 'error') return p.message || 'Cannot connect';
  if (p.state === 'connecting') return 'Connecting…';
  if (p.state === 'offline') return 'Offline — reconnecting';
  const stage = p.stage || '';
  if (ACTIVE.has(stage)) {
    const verb = stage === 'pause' ? 'Paused' : stage === 'prepare' ? 'Preparing' : 'Printing';
    return [`${verb}${p.job ? ` ${p.job}` : ''}`, p.progress != null && `${p.progress}%`, p.remainingMin ? `${fmtMin(p.remainingMin)} left` : ''].filter(Boolean).join(' · ');
  }
  if (stage === 'finish') return p.job ? `Finished ${p.job}` : 'Idle';
  if (stage === 'failed') return p.job ? `Failed: ${p.job}${p.error ? ` (error ${p.error})` : ''}` : 'Failed';
  return 'Idle';
}

const dotClass = (p: PrinterStatus) => (p.state !== 'online' ? (p.state === 'error' ? 'bad' : 'off') : p.stage === 'failed' || p.error ? 'bad' : ACTIVE.has(p.stage || '') ? 'on' : 'idle');

/** Dashboard top-bar button: shows the active print, if any. Hidden with no printers. */
export function PrinterButton({ printers, unassigned, on, onClick }: { printers: PrinterStatus[] | null; unassigned: number; on: boolean; onClick: () => void }) {
  if (!printers?.length) return null;
  const busy = printers.find((p) => p.state === 'online' && ACTIVE.has(p.stage || ''));
  const bad = printers.some((p) => p.state === 'error' || (p.state === 'online' && p.stage === 'failed'));
  const title = printers.map((p) => `${p.name}: ${printerLine(p)}`).join('\n') + (unassigned ? `\n${unassigned} print${unassigned === 1 ? '' : 's'} not matched to a project` : '');
  return (
    <button className={cx('btn dash-pop printer-pop', busy ? '' : 'icon', on && 'on', bad && 'alert')} title={title} aria-label="Printers" onClick={onClick}>
      <LuPrinter />
      {busy && <span className="pp-prog"><span className="pp-bar"><i style={{ width: `${busy.progress || 0}%` }} /></span>{busy.progress ?? 0}%{busy.remainingMin ? ` · ${fmtMin(busy.remainingMin)}` : ''}</span>}
      {unassigned > 0 && <span className="dash-badge">{unassigned}</span>}
    </button>
  );
}

function ResultIcon({ r }: { r: PrintRecord['result'] }) {
  return r === 'success' ? <LuCircleCheck color="var(--ok)" /> : r === 'stopped' ? <LuCircleSlash color="var(--fg2)" /> : <LuCircleX color="var(--bad)" />;
}

/** Drawer body: live status per printer, then recent prints (with Assign for unmatched ones). */
export function PrinterPanel({ printers, history, onChanged, nav }: { printers: PrinterStatus[] | null; history: PrintRecord[]; onChanged: () => void; nav: Nav }) {
  return (
    <>
      {(printers || []).map((p) => (
        <div key={p.id} className="pcard-live">
          <div className="row" style={{ gap: 8 }}><span className={`pdot ${dotClass(p)}`} /><b>{p.name}</b><span className="grow" />
            {p.state === 'online' && (p.nozzle != null || p.bed != null) && <span className="small dim">Nozzle {p.nozzle ?? '–'}° · Bed {p.bed ?? '–'}°</span>}</div>
          <div className={cx('small', p.state === 'error' ? 'bad-text' : 'muted')} style={{ marginTop: 4 }}>{printerLine(p)}</div>
          {p.state === 'online' && ACTIVE.has(p.stage || '') && <>
            <div className="pp-bar big"><i style={{ width: `${p.progress || 0}%` }} /></div>
            {p.totalLayers ? <div className="small dim">Layer {p.layer ?? 0} of {p.totalLayers}</div> : null}
          </>}
          {p.state === 'online' && !!p.hms && <div className="small bad-text" style={{ marginTop: 4 }}>The printer reports {p.hms} warning{p.hms === 1 ? '' : 's'} (see its screen).</div>}
        </div>
      ))}
      <h3 style={{ margin: '14px 0 6px' }}>Recent prints</h3>
      {history.length ? history.map((h) => <PrintRow key={h.printerId + h.key} h={h} onChanged={onChanged} nav={nav} />)
        : <div className="ds-empty"><LuPrinter /> Prints show up here when they finish, fail or are stopped, and are added to the matching project's print history.</div>}
    </>
  );
}

function PrintRow({ h, onChanged, nav }: { h: PrintRecord; onChanged: () => void; nav: Nav }) {
  const [assigning, setAssigning] = useState(false);
  const [projects, setProjects] = useState<Project[]>([]);
  const [pick, setPick] = useState('');
  const [remember, setRemember] = useState(true);
  const act = useAction();
  const open = async () => {
    const all = await call<Project[]>('projects.list');
    const sorted = [...all].sort((a, b) => Number(b.category === '3d-printing') - Number(a.category === '3d-printing') || a.name.localeCompare(b.name));
    setProjects(sorted);
    setPick(sorted[0]?.id || '');
    setAssigning(true);
  };
  const what = h.result === 'success' ? 'Printed' : h.result === 'stopped' ? 'Stopped' : 'Failed';
  return (
    <div className="print-row">
      <div className="row" style={{ gap: 8 }}>
        <ResultIcon r={h.result} />
        <span className="grow" style={{ minWidth: 0 }}><b>{h.name || 'Untitled job'}</b> <span className="small dim">{what}{h.error ? ` · error ${h.error}` : ''}{h.minutes ? ` · ${fmtMin(h.minutes)}` : ''}</span></span>
        <span className="small dim" title={h.printer}>{when(h.endedAt)}</span>
      </div>
      <div className="row small" style={{ gap: 8, marginTop: 3, paddingLeft: 24 }}>
        {h.projectId
          ? <a href="#" onClick={(e) => { e.preventDefault(); nav.go({ view: 'project', id: h.projectId! }); }}>{h.projectName}</a>
          : !assigning && <><span className="dim">No project matched.</span><button className="btn sm" onClick={open}>Assign…</button>
            <button className="btn ghost sm" title="Hide this print" onClick={async () => { if (await act(() => call('printers.assign', { key: h.key }))) onChanged(); }}>Dismiss</button></>}
      </div>
      {assigning && (
        <div className="row small" style={{ gap: 6, marginTop: 6, paddingLeft: 24, flexWrap: 'wrap' }}>
          <select className="input" style={{ width: 'auto', maxWidth: 220 }} value={pick} onChange={(e) => setPick(e.target.value)} aria-label="Project">
            {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
          <label className="row" style={{ gap: 4 }} title={`Later prints named "${h.name}" go to this project automatically`}><input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} /> Remember for "{h.name}"</label>
          <button className="btn primary sm" disabled={!pick} onClick={async () => { if (await act(() => call('printers.assign', { key: h.key, projectId: pick, remember }))) { setAssigning(false); onChanged(); } }}>Assign</button>
          <button className="btn ghost sm" onClick={() => setAssigning(false)}>Cancel</button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- settings

const EMPTY = { id: '', name: '', host: '', serial: '', accessCode: '' };

export function PrinterSettings() {
  const printers = usePrinters();
  const [form, setForm] = useState<typeof EMPTY | null>(null);
  const act = useAction();
  const confirm = useConfirm();
  const save = async () => {
    if (!form) return;
    if (await act(() => call('printers.save', form))) setForm(null);
  };
  return (
    <div className="card pad">
      <h2>Bambu Lab printers</h2>
      <p className="muted small" style={{ marginTop: 0 }}>
        Mothball can watch a Bambu Lab printer on your network: what it's printing, how far along it is, and whether each print finished or failed.
        Finished and failed prints are added to the print history of the project whose model or plate file has the same name. Mothball only reads the
        printer's status — it never starts, stops or changes anything, and it talks to the printer directly, not through the internet.
      </p>
      {(printers || []).map((p) => (
        <div key={p.id} className="printer-set-row">
          <span className={`pdot ${dotClass(p)}`} />
          <div className="grow" style={{ minWidth: 0 }}>
            <div><b>{p.name}</b> <span className="small dim mono">{p.host} · {p.serial}</span></div>
            <div className={cx('small', p.state === 'error' ? 'bad-text' : 'muted')}>{printerLine(p)}</div>
          </div>
          {p.certPinned && p.state === 'error' && /different security certificate/.test(p.message || '') && <button className="btn ghost sm" title="Use this if the printer's firmware was updated and it now presents a new certificate" onClick={async () => {
            if (await confirm({ title: 'Trust a new certificate?', body: 'The next certificate the printer presents will be remembered. Only do this if the printer\'s firmware was just updated or it was reset.', confirm: 'Trust new certificate' })) await act(() => call('printers.resetCertificate', { id: p.id }));
          }}><LuShieldCheck /> Trust new certificate</button>}
          <button className="btn ghost sm icon" title="Edit" aria-label={`Edit ${p.name}`} onClick={() => setForm({ id: p.id, name: p.name, host: p.host, serial: p.serial, accessCode: '' })}><LuPencil /></button>
          <button className="btn ghost sm icon" title="Remove" aria-label={`Remove ${p.name}`} onClick={async () => {
            if (await confirm({ title: `Remove ${p.name}?`, body: 'Mothball stops watching it. Prints already logged stay in your projects.', confirm: 'Remove', danger: true })) await act(() => call('printers.remove', { id: p.id }));
          }}><LuTrash2 /></button>
        </div>
      ))}
      {form ? (
        <div className="printer-form">
          <div className="field"><label>Name</label><input className="input" value={form.name} placeholder="X2D" onChange={(e) => setForm({ ...form, name: e.target.value })} /></div>
          <div className="field"><label>IP address</label><input className="input mono" value={form.host} placeholder="192.168.1.50" onChange={(e) => setForm({ ...form, host: e.target.value })} /></div>
          <div className="field"><label>Serial number</label><input className="input mono" value={form.serial} placeholder="Printer serial number" onChange={(e) => setForm({ ...form, serial: e.target.value })} /></div>
          <div className="field"><label>LAN access code</label><input className="input mono" type="password" autoComplete="off" value={form.accessCode} placeholder={form.id ? 'Leave empty to keep the current code' : '8 characters, shown on the printer'} onChange={(e) => setForm({ ...form, accessCode: e.target.value })} /></div>
          <p className="small dim" style={{ gridColumn: '1 / -1', margin: 0 }}>
            On the printer's screen, the network settings show its IP address and the LAN access code. The serial number is in the printer's device information,
            and in Bambu Studio or Bambu Handy under the device. The access code is stored only in Mothball's database on this computer.
          </p>
          <div className="row" style={{ gridColumn: '1 / -1', justifyContent: 'flex-end' }}>
            <button className="btn" onClick={() => setForm(null)}>Cancel</button>
            <button className="btn primary" onClick={save}>{form.id ? 'Save' : 'Add printer'}</button>
          </div>
        </div>
      ) : <button className="btn" style={{ marginTop: 10 }} onClick={() => setForm({ ...EMPTY })}><LuPlus /> Add printer</button>}
    </div>
  );
}
