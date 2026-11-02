import { useEffect, useState } from 'react';
import { LuGitBranch, LuGitCommitHorizontal, LuChevronDown, LuChevronRight, LuInfo, LuGithub, LuUpload, LuDownload, LuRefreshCw, LuCheck, LuExternalLink } from 'react-icons/lu';
import { call, isTauri, type VFile } from '../ipc';
import { Empty, useAction, useConfirm, useLoad, useToast } from '../components/common';
import { slug } from '../format';
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
    <div className="page"><Empty icon={<LuGitBranch />} title={info.data?.gitAvailable === false ? 'Git is not installed' : 'This project isn’t using Git yet'}>
      {info.data?.gitAvailable === false
        ? <p style={{ maxWidth: 560 }}>Install <b>Git for Windows</b> (git-scm.com), restart Project Vault, and come back here to see history and push to GitHub.</p>
        : <>
          <p style={{ maxWidth: 560 }}>Git keeps a full history of your project and lets you back it up to GitHub. Project Vault already keeps its own file versions (see each file’s History), so this is optional — but it’s the way to put the project on GitHub.</p>
          <InitButton ctx={ctx} onDone={info.reload} />
        </>}
    </Empty></div>
  );
  const g = info.data;
  return (
    <div className="page" style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 380px', gap: 18, alignItems: 'start', maxWidth: 'none' }}>
      <div>
        <div className="alert info" style={{ marginBottom: 12 }}><LuInfo /><div>History of your Git repository. Project Vault only changes it when you commit, push or pull from the panel on the right — it never resets, rebases or force-pushes.</div></div>
        <h2 style={{ marginBottom: 10 }}>History</h2>
        {(log.data || []).map((c) => (
          <div key={c.hash} className="card" style={{ marginBottom: 6 }}>
            <div className="row" style={{ padding: '8px 12px', cursor: 'pointer' }} onClick={() => setOpen(open === c.hash ? null : c.hash)}>
              {open === c.hash ? <LuChevronDown /> : <LuChevronRight />}<LuGitCommitHorizontal className="dim" />
              <span className="mono dim">{c.short}</span><b className="grow" style={{ fontWeight: 500 }}>{c.subject}</b>
              <span className="small dim">{c.files.length} file{c.files.length === 1 ? '' : 's'}</span><span className="small dim" title={dateTime(c.time)}>{c.author} · {ago(c.time)}</span>
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
        <SyncPanel ctx={ctx} info={g} status={status.data || []} onChanged={() => { info.reload(); status.reload(); log.reload(); }} />
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

function InitButton({ ctx, onDone }: { ctx: ProjectCtx; onDone: () => void }) {
  const act = useAction();
  const confirm = useConfirm();
  return (
    <button className="btn primary" onClick={async () => {
      if (await confirm({ title: 'Start using Git here?', confirm: 'Create repository', body: <>This runs <code>git init</code> in <span className="mono">{ctx.project.rootPath}</span> and adds a basic <code>.gitignore</code> if there isn’t one. Your files are not changed or uploaded. You can then commit and connect GitHub.</> })) {
        if (await act(() => call('git.init', { projectId: ctx.project.id }), 'Git repository created')) onDone();
      }
    }}><LuGitBranch /> Start using Git</button>
  );
}

function githubWebUrl(remote?: string): string | null {
  if (!remote) return null;
  let m = /^git@github\.com:(.+?)(\.git)?$/.exec(remote);
  if (m) return `https://github.com/${m[1]}`;
  m = /^https:\/\/github\.com\/(.+?)(\.git)?$/.exec(remote);
  return m ? `https://github.com/${m[1]}` : null;
}

function SyncPanel({ ctx, info, status, onChanged }: { ctx: ProjectCtx; info: any; status: any[]; onChanged: () => void }) {
  const pid = ctx.project.id;
  const act = useAction();
  const toast = useToast();
  const identity = useLoad(() => call<{ name: string; email: string }>('git.identity', { projectId: pid }), [pid]);
  const gh = useLoad(() => (info.remote ? Promise.resolve(null) : call<any>('git.ghStatus', { projectId: pid })), [pid, info.remote]);
  const [msg, setMsg] = useState('');
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<string | null>(null);
  const [who, setWho] = useState({ name: '', email: '' });
  const [url, setUrl] = useState('');
  const [repoName, setRepoName] = useState(slug(ctx.project.name));
  const [priv, setPriv] = useState(true);
  useEffect(() => { setSel(new Set(status.map((s) => s.path))); }, [status]);
  const needsId = identity.data && (!identity.data.name || !identity.data.email);
  const run = async (label: string, fn: () => Promise<any>, ok: string) => {
    setBusy(label);
    const r = await act(fn, ok);
    setBusy(null);
    if (r) { onChanged(); ctx.reload(); }
    return r;
  };
  const web = githubWebUrl(info.remote);
  const ahead = info.upstream?.ahead ?? 0, behind = info.upstream?.behind ?? 0;

  return (
    <>
      <div className="card pad">
        <h2 className="row" style={{ marginBottom: 8 }}><LuCheck /> Commit changes</h2>
        {needsId ? <>
          <p className="small muted" style={{ marginTop: 0 }}>Git needs a name and email for your commits (saved for this repository only). Use the email of your GitHub account.</p>
          <input className="input" placeholder="Your name" value={who.name} onChange={(e) => setWho({ ...who, name: e.target.value })} style={{ marginBottom: 6 }} />
          <input className="input" placeholder="you@example.com" value={who.email} onChange={(e) => setWho({ ...who, email: e.target.value })} style={{ marginBottom: 6 }} />
          <button className="btn" onClick={async () => { if (await act(() => call('git.setIdentity', { projectId: pid, ...who }))) identity.reload(); }}>Save</button>
        </> : status.length === 0 ? <div className="small muted">Nothing to commit — everything is saved in Git.</div> : <>
          <div style={{ maxHeight: 180, overflow: 'auto', marginBottom: 8 }}>
            {status.map((s) => (
              <label key={s.path} className="row small" style={{ padding: '1px 0' }}>
                <input type="checkbox" checked={sel.has(s.path)} onChange={() => setSel((x) => { const n = new Set(x); n.has(s.path) ? n.delete(s.path) : n.add(s.path); return n; })} />
                <span className="mono" style={{ width: 22, color: s.code.includes('?') ? 'var(--ok)' : s.code.includes('D') ? 'var(--bad)' : 'var(--warn)' }}>{s.code.includes('?') ? 'new' : s.code}</span>
                <span className="mono grow" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.path}</span>
              </label>
            ))}
          </div>
          <textarea className="input" rows={2} placeholder="What did you change? e.g. “Tune instashoot timing”" value={msg} onChange={(e) => setMsg(e.target.value)} />
          <button className="btn primary" style={{ marginTop: 6 }} disabled={!msg.trim() || !sel.size || !!busy}
            onClick={async () => { const all = sel.size === status.length; if (await run('commit', () => call('git.commit', { projectId: pid, message: msg, paths: all ? [] : [...sel] }), 'Committed')) setMsg(''); }}>
            {busy === 'commit' ? 'Committing…' : `Commit ${sel.size} file${sel.size === 1 ? '' : 's'}`}
          </button>
        </>}
      </div>

      <div className="card pad">
        <h2 className="row" style={{ marginBottom: 8 }}><LuGithub /> GitHub</h2>
        {info.remote ? <>
          <div className="small" style={{ marginBottom: 8 }}>
            {web ? <a href="#" onClick={(e) => { e.preventDefault(); if (isTauri) import('@tauri-apps/plugin-opener').then((o: any) => o.openUrl(web)); else window.open(web, '_blank', 'noopener'); }}>{web.replace('https://', '')} <LuExternalLink size={11} /></a> : <span className="mono">{info.remote}</span>}
          </div>
          <div className="small muted" style={{ marginBottom: 8 }}>
            {!info.upstream ? 'Not pushed yet.' : ahead === 0 && behind === 0 ? 'Up to date with GitHub.' : [ahead && `${ahead} commit${ahead === 1 ? '' : 's'} to push`, behind && `${behind} new on GitHub`].filter(Boolean).join(' · ')}
          </div>
          <div className="row wrap">
            <button className="btn primary" disabled={!!busy || (!!info.upstream && ahead === 0)} onClick={() => run('push', () => call('git.push', { projectId: pid }), 'Pushed')}><LuUpload /> {busy === 'push' ? 'Pushing…' : 'Push'}</button>
            <button className="btn" disabled={!!busy || !info.upstream} onClick={() => run('pull', () => call('git.pull', { projectId: pid }), 'Pulled latest changes')}><LuDownload /> {busy === 'pull' ? 'Pulling…' : 'Pull'}</button>
            <button className="btn icon" title="Check GitHub for new commits" disabled={!!busy} onClick={() => run('fetch', () => call('git.fetch', { projectId: pid }), '')}><LuRefreshCw className={busy === 'fetch' ? 'spin' : ''} /></button>
          </div>
          <p className="small dim" style={{ marginBottom: 0 }}>The first push may open a GitHub sign-in window (Git Credential Manager). Pull only fast-forwards, so it never merges or overwrites your local commits.</p>
        </> : <>
          <p className="small muted" style={{ marginTop: 0 }}>Put this project on GitHub to back it up and share it.</p>
          {gh.data?.loggedIn ? <>
            <h3 style={{ marginBottom: 6 }}>Create a new GitHub repository</h3>
            <div className="row" style={{ marginBottom: 6 }}><input className="input mono" value={repoName} onChange={(e) => setRepoName(e.target.value)} /><label className="row small"><input type="checkbox" checked={priv} onChange={(e) => setPriv(e.target.checked)} /> Private</label></div>
            <button className="btn primary" disabled={!repoName || !!busy || !info.head} title={!info.head ? 'Commit something first' : undefined}
              onClick={() => run('create', () => call('git.ghCreate', { projectId: pid, name: repoName, private: priv, description: ctx.project.description }), 'Repository created and pushed')}><LuGithub /> {busy === 'create' ? 'Creating…' : 'Create & push'}</button>
            {!info.head && <div className="small dim">Make your first commit above, then create the repository.</div>}
            <h3 style={{ margin: '14px 0 6px' }}>…or connect an existing repository</h3>
          </> : <p className="small dim">
            Create an empty repository on <a href="#" onClick={(e) => { e.preventDefault(); const u = 'https://github.com/new'; if (isTauri) import('@tauri-apps/plugin-opener').then((o: any) => o.openUrl(u)); else window.open(u, '_blank', 'noopener'); }}>github.com/new</a> (don’t add a README), then paste its URL here.
            {gh.data && !gh.data.installed && <> With the GitHub CLI (<code>winget install GitHub.cli</code>, then <code>gh auth login</code>) Project Vault can create the repository for you.</>}
            {gh.data?.installed && !gh.data.loggedIn && <> Run <code>gh auth login</code> once and Project Vault can create repositories for you.</>}
          </p>}
          <div className="row"><input className="input mono" placeholder="https://github.com/you/repo.git" value={url} onChange={(e) => setUrl(e.target.value)} />
            <button className="btn" disabled={!url.trim() || !!busy} onClick={async () => { if (await run('remote', () => call('git.setRemote', { projectId: pid, url }), 'Connected — now press Push')) setUrl(''); }}>Connect</button></div>
        </>}
      </div>
    </>
  );
}
