import { useState } from 'react';
import { LuGitBranch, LuGitCommitHorizontal, LuChevronDown, LuChevronRight, LuInfo } from 'react-icons/lu';
import { call, type VFile } from '../ipc';
import { Empty, useLoad, useToast } from '../components/common';
import { ago, dateTime } from '../format';
import type { ProjectCtx } from './Project';

export function DiffView({ text }: { text: string }) {
  if (!text.trim()) return <div className="small muted">No textual differences (binary file or no changes).</div>;
  return (
    <pre className="preview diff">
      {text.split('\n').map((l, i) => {
        const c = l.startsWith('+++') || l.startsWith('---') || l.startsWith('diff ') || l.startsWith('index ') ? 'meta' : l.startsWith('+') ? 'add' : l.startsWith('-') ? 'del' : l.startsWith('@@') ? 'hunk' : '';
        return <div key={i} className={c}>{l || ' '}</div>;
      })}
    </pre>
  );
}

const STATUS_LABEL: Record<string, string> = { M: 'modified', A: 'added', D: 'deleted', R: 'renamed', C: 'copied', '??': 'untracked', U: 'conflict' };

export function GitTab({ ctx }: { ctx: ProjectCtx }) {
  const pid = ctx.project.id;
  const info = useLoad(() => call<any>('git.info', { projectId: pid }), [pid]);
  const isRepo = info.data?.isRepo;
  const status = useLoad(() => (isRepo ? call<any[]>('git.status', { projectId: pid }) : Promise.resolve([])), [pid, isRepo, ctx.scanTick]);
  const log = useLoad(() => (isRepo ? call<any[]>('git.log', { projectId: pid, limit: 200 }) : Promise.resolve([])), [pid, isRepo]);
  const [open, setOpen] = useState<string | null>(null);
  const [diff, setDiff] = useState<{ key: string; text: string } | null>(null);
  const toast = useToast();

  const showDiff = async (key: string, rev: string | null, path?: string) => {
    if (diff?.key === key) return setDiff(null);
    try { const r: any = await call('git.diff', { projectId: pid, rev, path }); setDiff({ key, text: r.diff + (r.truncated ? '\n… (diff truncated at 1 MB)' : '') }); } catch (e: any) { toast(e.message, 'error'); }
  };
  const openFileByPath = async (path: string) => {
    // git paths are relative to the repo top-level; the project root may be a subfolder
    const files = await call<VFile[]>('files.list', { projectId: pid, query: path.split('/').pop(), limit: 50 });
    const f = files.find((x) => path.endsWith(x.path));
    if (f) ctx.openFile(f.id); else toast('That file is not indexed in this project (ignored folder or outside the project root).');
  };

  if (info.loading && !info.data) return <div className="page muted">Loading…</div>;
  if (!isRepo) return (
    <div className="page"><Empty icon={<LuGitBranch />} title={info.data?.gitAvailable === false ? 'Git is not installed' : 'Not a Git repository'}>
      <p style={{ maxWidth: 560 }}>{info.data?.gitAvailable === false
        ? 'Install Git for Windows to see commit history, branches and diffs here. Project Vault uses your installed git in read-only mode.'
        : 'This project folder is not tracked by Git. Project Vault still records detected file changes in each file’s history and the timeline, but it does not keep copies of earlier file contents.'}</p>
    </Empty></div>
  );
  const g = info.data;
  return (
    <div className="page" style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 380px', gap: 18, alignItems: 'start', maxWidth: 'none' }}>
      <div>
        <div className="alert info" style={{ marginBottom: 12 }}><LuInfo /><div>Git commits shown here come from your repository. Project Vault only reads it — it never commits, checks out or resets.</div></div>
        <h2 style={{ marginBottom: 10 }}>History</h2>
        {(log.data || []).map((c) => (
          <div key={c.hash} className="card" style={{ marginBottom: 6 }}>
            <div className="row" style={{ padding: '8px 12px', cursor: 'pointer' }} onClick={() => setOpen(open === c.hash ? null : c.hash)}>
              {open === c.hash ? <LuChevronDown /> : <LuChevronRight />}<LuGitCommitHorizontal className="dim" />
              <span className="mono dim">{c.short}</span><b className="grow" style={{ fontWeight: 500 }}>{c.subject}</b>
              <span className="small dim">{c.files.length} files</span><span className="small dim" title={dateTime(c.time)}>{c.author} · {ago(c.time)}</span>
            </div>
            {open === c.hash && (
              <div style={{ padding: '0 12px 10px 40px' }}>
                {c.body && <pre className="small muted" style={{ whiteSpace: 'pre-wrap', margin: '0 0 8px' }}>{c.body}</pre>}
                {c.files.map((f: any) => (
                  <div key={f.path} className="row small" style={{ padding: '2px 0' }}>
                    <span className="pill" style={{ minWidth: 70, justifyContent: 'center' }}>{STATUS_LABEL[f.status[0]] || f.status}</span>
                    <a href="#" className="mono grow" onClick={(e) => { e.preventDefault(); openFileByPath(f.path); }}>{f.from ? `${f.from} → ${f.path}` : f.path}</a>
                    <button className="btn ghost sm" onClick={() => showDiff(c.hash + f.path, c.hash, f.path)}>Diff</button>
                  </div>
                ))}
                {diff && diff.key.startsWith(c.hash) && <DiffView text={diff.text} />}
              </div>
            )}
          </div>
        ))}
        {log.data && !log.data.length && <div className="muted">No commits yet.</div>}
      </div>
      <div className="col" style={{ gap: 14 }}>
        <div className="card pad">
          <div className="kv">
            <span>Branch</span><span className="row"><LuGitBranch /> <b>{g.branch}</b></span>
            {g.upstream && <><span>Upstream</span><span>{g.upstream.ahead} ahead · {g.upstream.behind} behind</span></>}
            {g.head && <><span>Last commit</span><span>{g.head.subject} <span className="dim">({ago(g.head.time)})</span></span></>}
            {g.remote && <><span>Remote</span><span className="mono small" style={{ wordBreak: 'break-all' }}>{g.remote}</span></>}
            <span>Repository</span><span className="mono small" style={{ wordBreak: 'break-all' }}>{g.topLevel}</span>
          </div>
        </div>
        <div className="card pad">
          <h2 style={{ marginBottom: 8 }}>Uncommitted changes <span className="badge">{status.data?.length || 0}</span></h2>
          {(status.data || []).map((s) => (
            <div key={s.path} className="row small" style={{ padding: '2px 0' }}>
              <span className="mono" style={{ width: 24, color: s.code.includes('?') ? 'var(--fg3)' : s.code.includes('D') ? 'var(--bad)' : 'var(--warn)' }}>{s.code}</span>
              <a href="#" className="mono grow" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} onClick={(e) => { e.preventDefault(); openFileByPath(s.path); }}>{s.path}</a>
              {!s.code.includes('?') && <button className="btn ghost sm" onClick={() => showDiff('wt' + s.path, null, s.path)}>Diff</button>}
            </div>
          ))}
          {status.data && !status.data.length && <div className="small muted">Working tree clean.</div>}
          {diff && diff.key.startsWith('wt') && <DiffView text={diff.text} />}
        </div>
      </div>
    </div>
  );
}
