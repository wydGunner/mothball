import { useEffect, useMemo, useState } from 'react';
import { LuPlay, LuSquare, LuPlus, LuTrash2 } from 'react-icons/lu';
import { call } from '../ipc';
import { Modal, useAction, useLoad } from '../components/common';
import { cx } from '../format';

interface Session { projectId: string; start: number; end: number; sources: ('activity' | 'timer' | 'manual')[] }
interface Entry { id: string; projectId: string; start: number; end: number; note: string; source: string }
interface Summary { estimate: boolean; totalMs: number; sessions: Session[]; entries: Entry[]; timer: { projectId: string; start: number; note: string } | null }

export const fmtHM = (ms: number) => {
  const m = Math.round(ms / 60_000);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60), r = m % 60;
  return r ? `${h} h ${r} min` : `${h} h`;
};
/** Next local midnight. Calendar-based, so 23- and 25-hour days (DST changes) are right. */
const nextDay = (t: number) => { const d = new Date(t); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() + 1); return d.getTime(); };
/** Local midnights for the last `n` days, oldest first. */
const lastDays = (n: number) => { const out: number[] = []; const d = new Date(); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() - (n - 1)); for (let i = 0; i < n; i++) { out.push(d.getTime()); d.setDate(d.getDate() + 1); } return out; };
const SOURCE: Record<string, string> = { activity: 'file activity', timer: 'timer', manual: 'logged' };

/** Running-timer clock shown in the project header. */
export function TimerChip({ projectId, tick }: { projectId: string; tick: number }) {
  const t = useLoad(() => call<Summary['timer']>('time.timer'), [tick]);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const i = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(i); }, []);
  if (!t.data || t.data.projectId !== projectId) return null;
  const s = Math.floor((now - t.data.start) / 1000);
  return <span className="timerchip" title="Timer running"><span className="rec" />{Math.floor(s / 3600)}:{String(Math.floor(s / 60) % 60).padStart(2, '0')}:{String(s % 60).padStart(2, '0')}</span>;
}

/** Overview card: time this week, a 14-day chart, the timer and recent sessions. */
export function TimeCard({ projectId, onTimerChange }: { projectId: string; onTimerChange?: () => void }) {
  const dayStarts = useMemo(() => lastDays(14), []);
  const since = dayStarts[0];
  const [tick, setTick] = useState(0);
  const sum = useLoad(() => call<Summary>('time.summary', { projectId, since }), [projectId, tick]);
  const [logging, setLogging] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const i = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(i); }, []);
  const act = useAction();
  const d = sum.data;
  const running = d?.timer && d.timer.projectId === projectId ? d.timer : null;

  const days = useMemo(() => {
    const out = dayStarts.map((day) => ({ day, ms: 0 }));
    const add = (start: number, end: number) => {
      let a = Math.max(start, since);
      // split across the days it touches; always moves forward
      while (a < end) {
        const b = Math.min(end, nextDay(a));
        const di = out.findIndex((x, i) => a >= x.day && (i === out.length - 1 || a < out[i + 1].day));
        if (di >= 0) out[di].ms += b - a;
        if (b <= a) break;
        a = b;
      }
    };
    for (const s of d?.sessions || []) add(s.start, s.end);
    if (running) add(running.start, now);
    return out;
  }, [d, since, dayStarts, running, now]);
  const week = days.slice(7).reduce((a, x) => a + x.ms, 0);
  const max = Math.max(...days.map((x) => x.ms), 60 * 60_000);

  const toggle = async () => {
    await act(() => call(running ? 'time.stop' : 'time.start', { projectId }));
    setTick((t) => t + 1); onTimerChange?.();
  };
  if (sum.error) return <div className="muted small">{sum.error}</div>;
  if (!d) return null;
  const sessions = d.sessions.slice(0, showAll ? 30 : 4);
  return (
    <>
      <div className="tc-top">
        <div><div className="tc-big">{fmtHM(week)}</div><div className="small dim">last 7 days · {fmtHM(d.totalMs + (running ? now - running.start : 0))} in total</div></div>
        <button className={cx('btn sm', running && 'on-rec')} onClick={toggle} title={running ? 'Stop the timer and record the time' : 'Start a timer for this project'}>
          {running ? <><LuSquare /> {fmtClock(now - running.start)}</> : <><LuPlay /> Start timer</>}
        </button>
      </div>
      <div className="tc-bars" aria-label="Time per day, last 14 days">
        {days.map((x) => (
          <div key={x.day} className="tc-bar" title={`${new Date(x.day).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}: ${x.ms ? fmtHM(x.ms) : 'nothing'}`}>
            <div style={{ height: `${Math.max(x.ms ? 6 : 0, (x.ms / max) * 100)}%` }} />
          </div>
        ))}
      </div>
      {sessions.length > 0 && <div className="tc-list">
        {sessions.map((s) => {
          const e = d.entries.find((x) => x.start === s.start && x.end === s.end);
          return (
            <div key={s.start} className="tc-row">
              <span className="grow tc-when" title={s.sources.map((x) => SOURCE[x]).join(' + ')}>{new Date(s.start).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })}, {new Date(s.start).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}
                <span className="dim small"> · {s.sources.map((x) => SOURCE[x]).join(' + ')}</span></span>
              <span className="tc-dur">{fmtHM(s.end - s.start)}</span>
              {e && <button className="btn ghost sm icon" title="Delete this entry" onClick={async () => { await act(() => call('time.delete', { id: e.id })); setTick((t) => t + 1); }}><LuTrash2 /></button>}
            </div>
          );
        })}
      </div>}
      <div className="row small" style={{ marginTop: 6 }}>
        <button className="btn ghost sm" onClick={() => setLogging(true)}><LuPlus /> Log time</button>
        {d.sessions.length > 4 && <button className="btn ghost sm" onClick={() => setShowAll(!showAll)}>{showAll ? 'Fewer' : `All ${d.sessions.length} sessions`}</button>}
        {d.estimate && <span className="dim tc-est" title="Includes time estimated from when this project’s files were saved: saves within 30 minutes of each other count as one work session.">includes file activity</span>}
      </div>
      {logging && <LogTime projectId={projectId} onClose={() => setLogging(false)} onDone={() => { setLogging(false); setTick((t) => t + 1); }} />}
    </>
  );
}

const fmtClock = (ms: number) => { const s = Math.floor(ms / 1000); return `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`; };

function LogTime({ projectId, onClose, onDone }: { projectId: string; onClose: () => void; onDone: () => void }) {
  const today = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  const [date, setDate] = useState(`${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`);
  const [from, setFrom] = useState(`${pad(Math.max(0, today.getHours() - 1))}:00`);
  const [hours, setHours] = useState('1');
  const [note, setNote] = useState('');
  const act = useAction();
  const h = Number(hours.replace(',', '.'));
  const valid = h > 0 && h <= 24 && /^\d{4}-\d{2}-\d{2}$/.test(date) && /^\d{2}:\d{2}$/.test(from);
  const save = async () => {
    const start = new Date(`${date}T${from}:00`).getTime();
    const r = await act(() => call('time.add', { projectId, start, end: start + Math.round(h * 3_600_000), note }));
    if (r) onDone();
  };
  return (
    <Modal title="Log time" onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={!valid} onClick={save}>Add</button></>}>
      <div className="grid2">
        <div className="field"><label>Date</label><input className="input" type="date" value={date} onChange={(e) => setDate(e.target.value)} /></div>
        <div className="field"><label>Started at</label><input className="input" type="time" value={from} onChange={(e) => setFrom(e.target.value)} /></div>
      </div>
      <div className="field"><label>Hours</label><input className="input" inputMode="decimal" value={hours} onChange={(e) => setHours(e.target.value)} style={{ width: 120 }} /></div>
      <div className="field"><label>What you did <span className="dim">(optional)</span></label><input className="input" value={note} onChange={(e) => setNote(e.target.value)} placeholder="Modelled the receiver, test print" /></div>
    </Modal>
  );
}
