import { useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  LuTriangleAlert, LuGitCompareArrows, LuPlus, LuTrash2, LuCircleCheck, LuCircleX, LuFileWarning, LuPencil,
  LuNotebookPen, LuListTodo, LuHistory, LuChevronRight, LuDiff, LuBug, LuImage, LuCode,
  LuInfo, LuFileClock, LuSlidersHorizontal, LuClock, LuFolder, LuFolderX, LuPlug,
} from 'react-icons/lu';
import { DepsBody, depsSummary, type Dep } from './Deps';
import { SpoolChip } from './Printers';
import { call, type Project } from '../ipc';
import { KindIcon, Modal, StatusPill, TagInput, useAction, useLoad, useConfirm, useToast } from '../components/common';
import { PictureDialog, useThumb } from './Dashboard';
import { ago, bytes, cx, dateTime } from '../format';
import { CompareDialog, RevealButton } from './Versions';
import { Retention } from './History';
import { TimeCard, fmtHM } from './TimeCard';
import { PanelBoard } from '../components/Panels';
import { WhyNote } from '../components/WhyNote';
import type { ProjectCtx } from './Project';
import { customOf, splitKey, typeKeyOf, typeLabel, withType, useTypes } from '../types';
import { TypeSelect } from '../components/TypeSelect';

type FieldDef = { key: string; label: string; multiline?: boolean; options?: string[]; placeholder?: string };
export const MODULES: Record<string, { title: string; fields: FieldDef[] }> = {
  software: { title: 'Software details', fields: [
    { key: 'repoUrl', label: 'Repository URL' }, { key: 'language', label: 'Languages / runtime' },
    { key: 'buildInstructions', label: 'Build instructions', multiline: true }, { key: 'dependencies', label: 'Dependencies', multiline: true },
    { key: 'releaseHistory', label: 'Release history', multiline: true, placeholder: 'v1.2 — 2026-08-01 — …' },
  ] },
  game: { title: 'Game details', fields: [
    { key: 'engine', label: 'Engine', placeholder: 'Unity' }, { key: 'engineVersion', label: 'Engine version', placeholder: '6000.0.x' },
    { key: 'platforms', label: 'Target platforms' }, { key: 'renderPipeline', label: 'Render pipeline' },
    { key: 'sceneNotes', label: 'Scenes & key scripts', multiline: true }, { key: 'buildNotes', label: 'Build notes', multiline: true },
    { key: 'playtestNotes', label: 'Playtesting notes', multiline: true },
  ] },
  '3d-printing': { title: 'Print details', fields: [
    { key: 'printStatus', label: 'Print status', options: ['', 'IDEA', 'MODELED', 'SOURCE_ONLY', 'SLICED_UNTESTED', 'TEST_FIT_PASSED', 'REPRINT_READY', 'NEEDS_WORK', 'FAILED_PRINT', 'ARCHIVED'] },
    { key: 'material', label: 'Material', placeholder: 'PLA' }, { key: 'filamentColor', label: 'Filament color(s)' },
    { key: 'filamentGrams', label: 'Filament (g)' }, { key: 'printTime', label: 'Print time', placeholder: '5h 30m' },
    { key: 'printer', label: 'Printer' }, { key: 'nozzle', label: 'Nozzle' }, { key: 'layerHeight', label: 'Layer height' },
    { key: 'slicer', label: 'Slicer' }, { key: 'slicerProfile', label: 'Slicer profile' }, { key: 'dimensions', label: 'Dimensions (mm)' },
    { key: 'fitsContains', label: 'Fits / contains' },
    { key: 'assemblyNotes', label: 'Assembly notes', multiline: true }, { key: 'finishingNotes', label: 'Finishing instructions', multiline: true },
  ] },
  addon: { title: 'Addon / mod details', fields: [
    { key: 'game', label: 'Game', placeholder: 'Which game it is for' }, { key: 'client', label: 'Game version / client' },
    { key: 'addonVersion', label: 'Version' }, { key: 'pageUrl', label: 'Download / project page' },
    { key: 'installPath', label: 'Where it goes in the game' },
    { key: 'requirements', label: 'What it needs to run', multiline: true }, { key: 'releaseNotes', label: 'Release notes', multiline: true },
  ] },
  'addon:wow': { title: 'WoW addon details', fields: [
    { key: 'game', label: 'Game / flavor', placeholder: 'World of Warcraft (Retail)' }, { key: 'interface', label: 'Interface version', placeholder: '110205' },
    { key: 'addonVersion', label: 'Version' }, { key: 'pageUrl', label: 'CurseForge / Wago / GitHub page' },
    { key: 'savedVariables', label: 'Saved variables', placeholder: 'MyAddonDB' },
    { key: 'dependencies', label: 'Libraries it needs', multiline: true, placeholder: 'LibStub, AceAddon-3.0, …' },
    { key: 'releaseNotes', label: 'Release notes', multiline: true },
  ] },
  'addon:iw4x': { title: 'IW4X mod details', fields: [
    { key: 'client', label: 'IW4X client build', placeholder: 'r4432' },
    { key: 'modKind', label: 'Kind of mod', options: ['', 'Mod (mods folder)', 'Raw scripts', 'Menu / UI', 'Map', 'Other'] },
    { key: 'installPath', label: 'Folder in the game', placeholder: 'mods\\omen' },
    { key: 'entryPoint', label: 'Entry point script', placeholder: 'scripts/mp/_load.gsc' },
    { key: 'gameModes', label: 'Game modes', placeholder: 'S&D, FFA' },
    { key: 'whoNeedsIt', label: 'Who needs it', options: ['', 'Host only', 'Everyone in the lobby', 'Dedicated server'] },
    { key: 'addonVersion', label: 'Version' }, { key: 'pageUrl', label: 'Download / project page' },
    { key: 'dvars', label: 'Dvars / server config', multiline: true, placeholder: 'set omen_instashoot 1' },
    { key: 'requirements', label: 'What it needs to run', multiline: true, placeholder: 'Attach DLLs and other files under Dependencies so Deploy and Launch put them in place' },
    { key: 'releaseNotes', label: 'Release notes', multiline: true },
  ] },
  video: { title: 'Video details', fields: [
    { key: 'editor', label: 'Editor', placeholder: 'Vegas Pro 21' }, { key: 'resolution', label: 'Resolution / frame rate', placeholder: '1920×1080 @ 60' },
    { key: 'length', label: 'Length' }, { key: 'published', label: 'Published at (URL)' },
    { key: 'mediaLocation', label: 'Where the source footage lives', placeholder: 'E:\\Recordings\\2026-10', multiline: true },
    { key: 'plugins', label: 'Plugins & effects used', multiline: true }, { key: 'exportSettings', label: 'Export settings', multiline: true },
  ] },
  motion: { title: 'Motion details', fields: [
    { key: 'software', label: 'Software', placeholder: 'After Effects 2025, Cinema 4D 2025' }, { key: 'resolution', label: 'Resolution / frame rate' },
    { key: 'length', label: 'Length' }, { key: 'renderer', label: 'Renderer', placeholder: 'Redshift' },
    { key: 'plugins', label: 'Plugins, presets & fonts needed to reopen', multiline: true }, { key: 'renderSettings', label: 'Render settings', multiline: true },
    { key: 'assetsLocation', label: 'Where outside assets live', multiline: true },
  ] },
  design: { title: 'Design details', fields: [
    { key: 'software', label: 'Software', placeholder: 'Photoshop 2025' }, { key: 'canvas', label: 'Canvas size', placeholder: '1280×720 px' },
    { key: 'colorMode', label: 'Colour mode', options: ['', 'RGB', 'CMYK', 'Grayscale'] }, { key: 'client', label: 'For / client' },
    { key: 'fonts', label: 'Fonts used', multiline: true }, { key: 'exports', label: 'Export sizes & formats', multiline: true },
  ] },
  '3d-art': { title: '3D art details', fields: [
    { key: 'software', label: 'Software', placeholder: 'Blender 4.4' }, { key: 'renderer', label: 'Renderer', placeholder: 'Cycles' },
    { key: 'polycount', label: 'Poly count' }, { key: 'output', label: 'Output', placeholder: 'Turntable, game asset, print…' },
    { key: 'addons', label: 'Add-ons & plugins needed to reopen', multiline: true }, { key: 'textures', label: 'Where textures & HDRIs live', multiline: true },
  ] },
  cad: { title: 'CAD details', fields: [
    { key: 'software', label: 'Software', placeholder: 'Fusion' }, { key: 'units', label: 'Units', options: ['', 'mm', 'cm', 'in'] },
    { key: 'revision', label: 'Revision' }, { key: 'material', label: 'Material' },
    { key: 'manufacturing', label: 'Manufacturing method', placeholder: '3D print, CNC, laser…' }, { key: 'tolerances', label: 'Tolerances & fits', multiline: true },
    { key: 'parameters', label: 'Key parameters / dimensions', multiline: true },
  ] },
  electronics: { title: 'Electronics details', fields: [
    { key: 'board', label: 'Board / PCB' }, { key: 'mcu', label: 'MCU' }, { key: 'firmware', label: 'Firmware' },
    { key: 'bom', label: 'Bill of materials', multiline: true }, { key: 'wiringNotes', label: 'Wiring / pinout notes', multiline: true },
  ] },
};

/** Detail fields for a type key ("addon:iw4x", a custom type, or a plain category). */
export function moduleFor(key: string): { title: string; fields: FieldDef[] } | undefined {
  const c = customOf(key);
  if (c) return { title: `${c.name} details`, fields: c.fields.map((f) => ({ key: f.key, label: f.label, multiline: f.multiline })) };
  if (MODULES[key]) return MODULES[key];
  // subtypes without their own fields ("Other game mod") use their type's
  return MODULES[splitKey(key).category];
}
export const fieldsFor = (key: string): FieldDef[] => moduleFor(key)?.fields || [];

/** Today as YYYY-MM-DD in local time (toISOString would give the UTC date). */
const localToday = () => { const d = new Date(); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };
/** A print log entry's date, in the same local format for printer and hand-logged entries. */
const logDate = (e: { at?: number; date?: string }) => {
  if (e.at) return new Date(e.at).toLocaleDateString();
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(e.date || '');
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toLocaleDateString() : e.date || '';
};
/** Grams shown beside an entry only when no filament chip already says it. */
const looseGrams = (e: { grams?: string; filaments?: { grams?: number }[] }) => (e.grams && !(e.filaments || []).some((f) => f.grams) ? e.grams : '');

const PRINT_STATUS_DOT: Record<string, string> = { REPRINT_READY: 'on', TEST_FIT_PASSED: 'on', SLICED_UNTESTED: 'half', NEEDS_WORK: 'half' };

export function Overview({ ctx }: { ctx: ProjectCtx }) {
  const p = ctx.project;
  const act = useAction();
  const confirm = useConfirm();
  const toast = useToast();
  const [editing, setEditing] = useState<null | 'general' | 'details' | 'files'>(null);
  const [compare, setCompare] = useState<{ fileId: string; name: string } | null>(null);
  const reviews = useLoad(() => call<any[]>('files.reviews', { projectId: p.id }), [p.id, ctx.scanTick]);
  // newest first: what you just did is on top
  const changed = useLoad(() => call<any[]>('files.list', { projectId: p.id, changedOnly: true, sort: 'changed', limit: 50 }), [p.id, ctx.scanTick]);
  const missing = useLoad(() => call<any[]>('files.list', { projectId: p.id, state: 'missing', limit: 5000 }), [p.id, ctx.scanTick]);
  // folders this mod type's game writes on its own (demos, logs…), for projects not ignoring them yet
  const ignoreSug = useLoad(() => call<{ subtype: string; folders: { rule: string; folder: string; files: number; missing: number }[] }>('projects.ignoreSuggestions', { id: p.id }), [p.id, ctx.scanTick]);
  const sugFolders = ignoreSug.data?.folders || [];
  const [openGroup, setOpenGroup] = useState<string | null>(null);
  // missing files grouped by folder: 3+ in one folder collapse into one row
  const missingGroups = useMemo(() => {
    const by = new Map<string, any[]>();
    for (const f of missing.data || []) {
      const d = f.path.includes('/') ? f.path.slice(0, f.path.lastIndexOf('/')) : '';
      if (!by.has(d)) by.set(d, []);
      by.get(d)!.push(f);
    }
    return [...by.entries()].map(([dir, files]) => ({ dir, files })).sort((a, b) => b.files.length - a.files.length || a.dir.localeCompare(b.dir));
  }, [missing.data]);
  const groups = useLoad(() => call<any[]>('files.groups', { projectId: p.id }), [p.id, ctx.scanTick]);
  const tasks = useLoad(() => call<any[]>('tasks.list', { projectId: p.id }), [p.id]);
  const notes = useLoad(() => call<any[]>('notes.tree', { projectId: p.id }), [p.id]);
  const timeline = useLoad(() => call<any[]>('timeline.list', { projectId: p.id, limit: 8 }), [p.id, ctx.scanTick]);
  const recent = useLoad(() => call<any[]>('files.list', { projectId: p.id, sort: 'changed', limit: 6 }), [p.id, ctx.scanTick]);
  // same picture as the dashboard card (dark or tiny automatic picks are skipped)
  const thumb = useThumb(p);
  const [picking, setPicking] = useState(false);

  useTypes();
  const tkey = typeKeyOf(p.category, p.module);
  const mod = moduleFor(tkey);
  const filled = mod ? mod.fields.filter((f) => (p.module?.[f.key] || '').toString().trim()) : [];
  const custom = Object.entries((p.module?.custom || {}) as Record<string, string>).filter(([, v]) => v);
  const printLog: any[] = p.module?.printLog || [];
  const nextTasks = (tasks.data || []).filter((t) => t.status !== 'done').slice(0, 5);
  const attention = (reviews.data?.length || 0) + (changed.data?.length || 0) + (missing.data?.length || 0) + (sugFolders.length ? 1 : 0);
  const ignoreFolders = async (dirs: { dir: string; anywhere: boolean }[], title: string, body: ReactNode) => {
    if (!(await confirm({ title, confirm: 'Stop tracking', body }))) return;
    let kept = 0, done = false;
    await act(async () => {
      for (const d of dirs) kept += (await call<{ kept: number }>('files.ignoreFolder', { projectId: p.id, dir: d.dir, anywhere: d.anywhere })).kept;
      done = true;
    });
    if (done) toast(kept ? `No longer tracked. ${kept} file${kept === 1 ? '' : 's'} with your notes or backups ${kept === 1 ? 'was' : 'were'} kept; forget ${kept === 1 ? 'it' : 'them'} yourself if not needed.` : 'No longer tracked', 'success');
    missing.reload(); ignoreSug.reload(); ctx.reload();
  };
  const forgetMany = async (files: any[], where: string) => {
    const noted = files.filter((f) => f.hasDocs).length;
    if (await confirm({ title: `Forget ${files.length} missing files?`, danger: true, confirm: 'Forget all', body: <>The records for the {files.length} missing files in <span className="mono">{where}</span>{noted ? <>, including notes on {noted} of them,</> : ''} are removed from the vault. Nothing on disk changes. New files there will still be tracked.</> })) {
      await act(() => call('files.forgetMissingMany', { ids: files.map((f) => f.id) })); missing.reload(); ctx.reload();
    }
  };
  const recentNotes = [...(notes.data || [])].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 5);
  const openCount = (tasks.data || []).filter((t) => t.status !== 'done').length;
  const timeSum = useLoad(() => call<{ totalMs: number }>('time.summary', { projectId: p.id, since: Date.now() }), [p.id, ctx.scanTick]);
  const timeTotal = timeSum.data?.totalMs || 0;
  const [depsTick, setDepsTick] = useState(0);
  const deps = useLoad(() => call<Dep[]>('deps.list', { projectId: p.id }), [p.id, ctx.scanTick, depsTick]);
  const depsRelevant = ['addon', 'game', 'software'].includes(p.category) || !!deps.data?.length;

  const ext = (path: string) => (path.includes('.') ? path.split('.').pop()!.toLowerCase() : '');
  return (
    <div className="page ov2">
      <PanelBoard storageKey="overview" defaults={{ main: ['about', 'attention', 'recent', 'details', 'deps'], rail: ['time', 'next', 'notes', 'activity', 'info'] }}
        zoneClass={{ main: 'ov2-main', rail: 'ov2-rail' }} panels={[
        { id: 'about', icon: <LuInfo />, summary: p.description ? p.description.split('\n')[0] : undefined, title: 'About', className: 'ov2-hero-panel', body: <div className="ov2-hero">
          {thumb && <button className="ov2-pic" title="Change the picture" onClick={() => setPicking(true)}><img src={thumb.src} alt="" style={{ objectFit: thumb.fit || 'cover', objectPosition: `${thumb.x ?? 50}% ${thumb.y ?? 50}%` }} /></button>}
          <div className="ov2-about">
            {p.description
              ? <p className="ov-desc">{p.description}</p>
              : <p className="muted" style={{ margin: 0 }}><a href="#" onClick={(e) => { e.preventDefault(); setEditing('general'); }}>Add a description</a> of what this project is.</p>}
            <div className="ov2-facts">
              <span>{typeLabel(tkey)}</span>
              {p.status !== 'Active' && <StatusPill status={p.status} />}
              {p.group && <span>in {p.group}</span>}
              {p.tags.map((t) => <span className="tag" key={t}>{t}</span>)}
            </div>
            {p.progress > 0 && <div className="ov2-progress" title={`${p.progress}% done`}><div className="progress"><div style={{ width: `${p.progress}%` }} /></div><span className="small dim">{p.progress}%</span></div>}
            <div className="row" style={{ gap: 6, marginTop: 10 }}>
              <button className="btn sm" onClick={() => setEditing('general')}><LuPencil /> Edit project</button>
              {!thumb && <button className="btn sm ghost" onClick={() => setPicking(true)}><LuImage /> Add a picture</button>}
            </div>
          </div>
        </div> },
        { id: 'attention', icon: <LuTriangleAlert />, title: 'Needs your attention', summary: `${attention} item${attention === 1 ? '' : 's'} to look at`, meta: attention || undefined, hidden: attention === 0, className: 'ov2-sec attention', body: <>
            {(reviews.data || []).map((r) => (
              <div key={r.id} className="att-row">
                <LuGitCompareArrows />
                <div className="grow small">
                  <div><b>Is this the same file?</b> <span className="mono">{r.missing.path}</span> disappeared and <span className="mono">{r.candidate.path}</span> appeared <span className="dim">({r.reason})</span>.</div>
                  {r.missing.hasDocs && <div className="dim">Confirming moves its notes and history to the new path.</div>}
                </div>
                <button className="btn sm" onClick={async () => { await act(() => call('files.resolveReview', { id: r.id, accept: true }), 'Linked — notes now follow the file'); reviews.reload(); ctx.reload(); }}><LuCircleCheck /> Same file</button>
                <button className="btn sm" onClick={async () => { await act(() => call('files.resolveReview', { id: r.id, accept: false })); reviews.reload(); ctx.reload(); }}><LuCircleX /> Different</button>
              </div>
            ))}
            {(changed.data || []).map((f) => (
              <div key={f.id} className="att-row">
                <LuFileWarning />
                <div className="grow small"><span className="mono">{f.path}</span> <span className="dim">changed {ago(f.lastChanged)}</span>{f.hasDocs && <span className="dim">, has notes: check they still apply</span>}</div>
                {f.versions > 1 && <button className="btn sm" onClick={() => setCompare({ fileId: f.id, name: f.name })}><LuDiff /> What changed?</button>}
                <button className="btn sm ghost" onClick={() => ctx.openFile(f.id)}>Open</button>
                <button className="btn sm ghost" onClick={async () => { await act(() => call('files.markReviewed', { ids: [f.id] })); changed.reload(); ctx.reload(); }}>Dismiss</button>
              </div>
            ))}
            {sugFolders.length > 0 && (
              <div className="att-row att-suggest">
                <LuFolderX />
                <div className="grow small">
                  <div><b>Stop tracking game output?</b> {typeLabel(tkey)} writes {sugFolders.map((f, i) => <span key={f.rule}>{i ? ', ' : ''}<span className="mono">{f.folder}/</span></span>)} on its own ({sugFolders.reduce((n, f) => n + f.files, 0)} files{sugFolders.some((f) => f.missing) ? `, ${sugFolders.reduce((n, f) => n + f.missing, 0)} already gone` : ''}).</div>
                  <div className="dim">Match demos, logs, crash dumps and screenshots come and go as you play. Your binds and settings in <span className="mono">players/</span> stay tracked.</div>
                </div>
                <button className="btn sm" onClick={() => ignoreFolders(sugFolders.map((f) => ({ dir: f.folder, anywhere: true })), 'Stop tracking game output?',
                  <>Mothball stops tracking every {sugFolders.map((f, i) => <span key={f.rule}>{i ? ', ' : ''}<span className="mono">{f.folder}/</span></span>)} folder in this project. Nothing on disk changes, and files with your notes or backups are kept. To undo it, remove the rule under “Folders and file types to skip” when editing the project.</>)}><LuFolderX /> Stop tracking</button>
                <button className="btn sm ghost" onClick={async () => { await act(() => call('projects.declineIgnoreSuggestions', { id: p.id, rules: sugFolders.map((f) => f.rule) })); ignoreSug.reload(); }}>Keep tracking</button>
              </div>
            )}
            {missingGroups.filter((g) => g.files.length >= 3).map((g) => {
              const where = g.dir ? `${g.dir}/` : 'the project folder';
              const isOpen = openGroup === g.dir;
              return (
                <div key={'g:' + g.dir}>
                  <div className="att-row">
                    <LuTriangleAlert />
                    <div className="grow small"><span className="mono">{where}</span> <span className="dim">{g.files.length} files missing from disk</span></div>
                    <RevealButton open={isOpen} onClick={() => setOpenGroup(isOpen ? null : g.dir)} label={isOpen ? 'Hide files' : 'Show files'} />
                    <button className="btn sm" onClick={() => forgetMany(g.files, where)}>Forget all</button>
                    {g.dir && <button className="btn sm" title="Stop tracking this folder: its files won't show up here again" onClick={() => ignoreFolders([{ dir: g.dir, anywhere: false }], `Stop tracking ${where}?`,
                      <>Mothball stops tracking <span className="mono">{where}</span> and everything in it, including files added there later. Nothing on disk changes, and files with your notes or backups are kept. To undo it, remove the rule under “Folders and file types to skip” when editing the project.</>)}><LuFolderX /> Ignore folder</button>}
                  </div>
                  {isOpen && <div className="att-sub">{g.files.map((f) => (
                    <div key={f.id} className="att-row small">
                      <span className="mono grow">{f.name}</span>
                      <button className="btn sm ghost" onClick={() => ctx.openFile(f.id)}>Open</button>
                    </div>
                  ))}</div>}
                </div>
              );
            })}
            {missingGroups.filter((g) => g.files.length < 3).flatMap((g) => g.files).map((f) => (
              <div key={f.id} className="att-row">
                <LuTriangleAlert />
                <div className="grow small"><span className="mono">{f.path}</span> <span className="dim">is missing from disk</span>{f.hasDocs && <span className="dim">; its notes are kept</span>}</div>
                <button className="btn sm ghost" onClick={() => ctx.openFile(f.id)}>Open</button>
                <button className="btn sm ghost" onClick={async () => {
                  if (await confirm({ title: 'Forget this file?', danger: true, confirm: 'Forget', body: <>The record for <span className="mono">{f.path}</span>{f.hasDocs ? ', including its notes,' : ''} is removed from the vault. Nothing on disk changes.</> })) {
                    await act(() => call('files.forgetMissing', { id: f.id })); missing.reload(); ctx.reload();
                  }
                }}>Forget</button>
              </div>
            ))}
            {(changed.data?.length || 0) > 1 && <button className="btn sm" style={{ marginTop: 6 }} onClick={async () => { await act(() => call('files.markReviewed', { ids: changed.data!.map((f) => f.id) })); changed.reload(); ctx.reload(); }}>Dismiss all changes</button>}
        </> },
        { id: 'recent', icon: <LuFileClock />, summary: recent.data?.[0] ? <>{recent.data[0].name} · {ago(recent.data[0].lastChanged)}</> : undefined, title: 'Recently changed', actions: <button className="btn sm ghost" onClick={() => ctx.openFiles()}>All {p.fileCount} {p.fileCount === 1 ? 'file' : 'files'} <LuChevronRight /></button>, body: <>
          {recent.data?.length ? <div className="ov2-files">
            {recent.data.map((f) => {
              const d = f.path.includes('/') ? f.path.slice(0, f.path.lastIndexOf('/') + 1) : '';
              const canEdit = ['gsc', 'csc', 'gsh', 'lua', 'cfg', 'json', 'txt', 'md', 'cs', 'js', 'ts', 'py', 'ini', 'xml', 'menu', 'toc'].includes(ext(f.path));
              return (
                <div key={f.id} className="ov2-file" onClick={() => ctx.openFile(f.id)}>
                  <KindIcon kind={f.kind} />
                  <span className="mono name">{f.name}</span>
                  <span className="mono dim dir">{d}</span>
                  <span className="dim small when">{ago(f.lastChanged)}</span>
                  {canEdit && <button className="btn ghost sm" title="Edit in Mothball" onClick={(e) => { e.stopPropagation(); ctx.nav.go({ view: 'project', id: p.id, tab: 'files', focus: { type: 'file', id: f.id, action: 'edit', nonce: Date.now() } }); }}><LuCode /></button>}
                </div>
              );
            })}
          </div> : <div className="small muted">{p.fileCount ? 'No files yet.' : p.mode === 'managed' ? 'No files yet. Import some from the Files tab.' : 'No files found in the folder yet. Press Rescan.'}</div>}
          {(groups.data || []).some((g) => g.count > 0) && <div className="ov2-groups">
            {groups.data!.filter((g) => g.count > 0).map((g) => (
              <button key={g.id} className="chip" onClick={() => ctx.openFiles(g.id)} title={`${g.hint ? g.hint + ': ' : ''}${(g.found || []).map((f: any) => `${f.ext ? '.' + f.ext : 'no extension'} ${f.count}`).join(', ')}`}>{g.label} <span>{g.count}</span></button>
            ))}
          </div>}
        </> },
        { id: 'details', icon: <LuSlidersHorizontal />, summary: filled.length || custom.length ? `${filled.length + custom.length} filled in` : 'Nothing filled in yet', title: mod?.title || 'Details', hidden: !mod, actions: <button className="btn sm ghost" onClick={() => setEditing('details')}><LuPencil /> Edit</button>, body: <>
            {filled.length === 0 && !custom.length ? (
              <p className="muted small" style={{ margin: 0 }}><a href="#" onClick={(e) => { e.preventDefault(); setEditing('details'); }}>Add {p.category === '3d-printing' ? 'material, printer and print time' : 'details'}</a>{p.category === '3d-printing' ? ', or fill them from a .3mf in this project.' : '.'}</p>
            ) : (
              <>
                <dl className="ov2-dl">
                  {filled.filter((f) => !f.multiline).map((f) => (
                    <div key={f.key}><dt>{f.label}</dt>
                      {f.key === 'printStatus'
                        ? <dd><span className="pill"><span className={`dot ${PRINT_STATUS_DOT[p.module[f.key]] || 'off'}`} style={p.module[f.key] === 'FAILED_PRINT' ? { background: 'var(--bad)' } : undefined} />{p.module[f.key].replace(/_/g, ' ').toLowerCase()}</span></dd>
                        : <dd>{p.module[f.key]}</dd>}
                    </div>
                  ))}
                  {custom.map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}
                </dl>
                {filled.filter((f) => f.multiline).map((f) => (
                  <div key={f.key} style={{ marginTop: 12 }}><h3>{f.label}</h3><div className="small" style={{ whiteSpace: 'pre-wrap', marginTop: 4 }}>{p.module[f.key]}</div></div>
                ))}
              </>
            )}
            {p.category === '3d-printing' && printLog.length > 0 && (
              <div style={{ marginTop: 14 }}>
                <h3>Print history <span className="dim small" style={{ fontWeight: 400 }}>{printLog.filter((e) => e.result === 'success').length} succeeded, {printLog.filter((e) => e.result !== 'success').length} failed</span></h3>
                {printLog.slice(-4).reverse().map((e, i) => (
                  <div key={i} className="row small" style={{ padding: '3px 0' }}>{e.result === 'success' ? <LuCircleCheck color="var(--ok)" /> : <LuCircleX color="var(--bad)" />}<span style={{ width: 90 }} title={e.source === 'printer' ? 'Logged automatically from the printer' : undefined}>{logDate(e)}</span><span className="grow muted">{e.notes}</span>{(e.filaments || []).map((f: any, j: number) => <SpoolChip key={j} s={f} />)}{looseGrams(e) && <span className="dim">{looseGrams(e)} g</span>}</div>
                ))}
              </div>
            )}
        </> },
        { id: 'deps', icon: <LuPlug />, title: 'Dependencies', summary: depsSummary(deps.data), hidden: !depsRelevant, body: <DepsBody projectId={p.id} deps={deps.data} onChanged={() => setDepsTick((t) => t + 1)} /> },
        { id: 'time', icon: <LuClock />, summary: timeTotal ? `${fmtHM(timeTotal)} in total` : 'No time recorded yet', title: 'Time', body: <>
          <TimeCard projectId={p.id} onTimerChange={() => window.dispatchEvent(new CustomEvent('pv:timer'))} />
        </> },
        { id: 'next', icon: <LuListTodo />, summary: openCount ? `${openCount} open${nextTasks[0] ? ` · ${nextTasks[0].title}` : ''}` : 'Nothing open', title: 'Next up', actions: <button className="btn sm ghost" onClick={() => ctx.openTask('')}>Board <LuChevronRight /></button>, body: <>
          {nextTasks.length ? nextTasks.map((t) => (
            <div key={t.id} className="linkrow" onClick={() => ctx.openTask(t.id)}>
              {t.kind === 'bug' ? <LuBug color="var(--accent)" /> : <span className={`prio p${t.priority}`} title={['Low', 'Normal', 'High', 'Urgent'][t.priority] + ' priority'} />}
              <span className="grow">{t.title}</span><span className="dim small">{t.status.replace('_', ' ')}</span>
            </div>
          )) : <div className="muted small"><a href="#" onClick={(e) => { e.preventDefault(); ctx.openTask(''); }}>Add a task</a> so you know where to pick up next time.</div>}
        </> },
        { id: 'notes', icon: <LuNotebookPen />, summary: (notes.data?.length || 0) ? `${notes.data!.length} note${notes.data!.length === 1 ? '' : 's'}${recentNotes[0] ? ` · ${recentNotes[0].title}` : ''}` : 'No notes yet', title: 'Notes', actions: <button className="btn sm ghost" onClick={() => ctx.openNote('')}>All <LuChevronRight /></button>, body: <>
          {recentNotes.length ? recentNotes.map((n) => (
            <div key={n.id} className="linkrow" onClick={() => ctx.openNote(n.id)}><span className="grow">{n.title}</span><span className="dim small">{ago(n.updatedAt)}</span></div>
          )) : <div className="muted small"><a href="#" onClick={(e) => { e.preventDefault(); ctx.openNote(''); }}>Write the first note</a>: setup steps, decisions, how things fit together.</div>}
        </> },
        { id: 'activity', icon: <LuHistory />, summary: timeline.data?.[0] ? <>{timeline.data[0].summary} · {ago(timeline.data[0].at)}</> : undefined, title: 'Activity', actions: <button className="btn sm ghost" onClick={() => ctx.openTimeline()}>Timeline <LuChevronRight /></button>, body: <>
          {(timeline.data || []).map((e) => (
            <div key={e.id} className="why-host small" style={{ padding: '3px 0' }} title={e.note ? `Why: ${e.note}` : undefined}>
              <div className="row" style={{ alignItems: 'baseline' }}><span className="grow" style={e.manual ? { fontWeight: 600 } : undefined}>{e.summary}</span><span className="dim" style={{ whiteSpace: 'nowrap' }}>{ago(e.at)}</span></div>
              <WhyNote note={e.note} onSave={async (n) => { await call('timeline.setNote', { id: e.id, note: n }); timeline.reload(); }} />
            </div>
          ))}
        </> },
        { id: 'info', icon: <LuFolder />, summary: <span className="mono">{p.rootPath}</span>, title: 'Folder', className: 'small', body: <>

          <div className="kv">
            <span>Folder</span><span className="mono" style={{ wordBreak: 'break-all' }}>{p.rootPath}</span>
            <span>Files</span><span>{p.mode === 'linked' ? 'Edited where they are' : 'Kept in app storage'}</span>
            <span>Size</span><span>{p.fileCount} {p.fileCount === 1 ? 'file' : 'files'}, {bytes(p.bytes)}</span>
            <span>Created</span><span>{dateTime(p.createdAt)}</span>
          </div>
        </> },
      ]} />

      {picking && <PictureDialog p={p} onClose={() => setPicking(false)} onSaved={() => { setPicking(false); ctx.reload(); ctx.nav.refreshProjects(); }} />}
      {editing && <EditProjectDialog ctx={ctx} initialTab={editing} onClose={() => setEditing(null)} />}
      {compare && <CompareDialog fileId={compare.fileId} title={compare.name} onClose={() => setCompare(null)} onRestored={() => { ctx.scan(true); }} />}
    </div>
  );
}

// ---------------------------------------------------------------- edit dialog

export function EditProjectDialog({ ctx, initialTab, onClose }: { ctx: ProjectCtx; initialTab: 'general' | 'details' | 'files'; onClose: () => void }) {
  const p = ctx.project;
  const act = useAction();
  const toast = useToast();
  const [tab, setTab] = useState(initialTab);
  const [f, setF] = useState({ name: p.name, description: p.description, icon: p.icon, status: p.status, category: p.category, progress: p.progress, tags: p.tags, ignoreGlobs: p.ignoreGlobs, group: p.group || '' });
  const [module, setModule] = useState<Record<string, any>>(p.module || {});
  const [customStatus, setCustomStatus] = useState('');
  const statuses = useLoad(() => call<{ name: string }[]>('projects.statuses'), []);
  const policy = useLoad(() => call<any>('versions.policy', { projectId: p.id }), [p.id]);
  const stats = useLoad(() => call<any>('versions.stats', { projectId: p.id }), [p.id]);
  const [pol, setPol] = useState<any>(null);
  useEffect(() => { if (policy.data && !pol) setPol(policy.data); }, [policy.data, pol]);
  useTypes();
  const ekey = typeKeyOf(f.category, module);
  const mod = moduleFor(ekey);

  const save = async () => {
    const r = await act(() => call<Project>('projects.update', { id: p.id, patch: { ...f, status: customStatus.trim() || f.status, module } }));
    if (!r) return;
    if (pol) await act(() => call('versions.setPolicy', { projectId: p.id, policy: pol }));
    toast('Project saved', 'success');
    await ctx.reload();
    ctx.nav.refreshProjects();
    if (f.ignoreGlobs !== p.ignoreGlobs || f.category !== p.category) ctx.scan(true);
    onClose();
  };

  return (
    <Modal wide title={`Edit “${p.name}”`} onClose={onClose} footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" onClick={save}>Save</button></>}>
      <div className="seg" style={{ marginBottom: 16 }}>
        <button className={cx(tab === 'general' && 'on')} onClick={() => setTab('general')}>General</button>
        <button className={cx(tab === 'details' && 'on')} onClick={() => setTab('details')}>{mod ? mod.title : 'Details'}</button>
        <button className={cx(tab === 'files' && 'on')} onClick={() => setTab('files')}>Files & versions</button>
      </div>

      {tab === 'general' && <>
        <div className="grid2">
          <div className="field"><label>Name</label><input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} /></div>
          <div className="field"><label>Icon (an emoji, optional)</label><input className="input" maxLength={4} value={f.icon} onChange={(e) => setF({ ...f, icon: e.target.value })} placeholder="🛠" /></div>
        </div>
        <div className="field"><label>Description</label><textarea className="input" rows={3} value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} placeholder="What is this project and why does it exist?" /></div>
        <div className="grid3">
          <div className="field"><label>Status</label>
            <select className="input" value={f.status} onChange={(e) => setF({ ...f, status: e.target.value })}>{[...new Set([...(statuses.data || []).map((s) => s.name), f.status])].map((s) => <option key={s}>{s}</option>)}</select>
          </div>
          <div className="field"><label>…or a custom status</label><input className="input" value={customStatus} onChange={(e) => setCustomStatus(e.target.value)} placeholder="e.g. Waiting on parts" /></div>
          <div className="field"><label>Project type</label>
            <TypeSelect value={ekey} keep={typeKeyOf(p.category, p.module)} onChange={(k) => { setF({ ...f, category: splitKey(k).category }); setModule(withType(module, k)); }} />
          </div>
        </div>
        <div className="field"><label>Progress — {f.progress}%</label><input type="range" min={0} max={100} step={5} value={f.progress} onChange={(e) => setF({ ...f, progress: Number(e.target.value) })} /></div>
        <div className="grid2">
          <div className="field"><label>Group</label>
            <input className="input" list="pv-groups" value={f.group} maxLength={60} placeholder="e.g. MW2 mods (optional)" onChange={(e) => setF({ ...f, group: e.target.value })} />
            <datalist id="pv-groups">{[...new Set(ctx.nav.projects.map((x) => x.group).filter(Boolean))].map((g) => <option key={g} value={g} />)}</datalist>
          </div>
          <div className="field"><label>Tags</label><TagInput value={f.tags} onChange={(tags) => setF({ ...f, tags })} /></div>
        </div>
      </>}

      {tab === 'details' && <>
        {f.category === '3d-printing' && <FillFrom3mf projectId={p.id} module={module} onFill={setModule} />}
        {f.category === 'addon' && (ekey === 'addon:wow' || ekey === 'addon') && <FillFromToc projectId={p.id} module={module} onFill={(m) => setModule({ ...m, subtype: 'wow' })} />}
        {mod ? <>
          <div className="grid2">
            {(mod?.fields || []).filter((x) => !x.multiline).map((x) => (
              <div className="field" key={x.key}><label>{x.label}</label>
                {x.options
                  ? <select className="input" value={module[x.key] || ''} onChange={(e) => setModule({ ...module, [x.key]: e.target.value })}>{x.options.map((o) => <option key={o} value={o}>{o ? o.replace(/_/g, ' ').toLowerCase() : '—'}</option>)}</select>
                  : <input className="input" value={module[x.key] || ''} placeholder={x.placeholder} onChange={(e) => setModule({ ...module, [x.key]: e.target.value })} />}
              </div>
            ))}
          </div>
          {(mod?.fields || []).filter((x) => x.multiline).map((x) => (
            <div className="field" key={x.key}><label>{x.label}</label><textarea className="input" rows={3} value={module[x.key] || ''} placeholder={x.placeholder} onChange={(e) => setModule({ ...module, [x.key]: e.target.value })} /></div>
          ))}
        </> : <p className="muted small">This project type has no special fields. Use custom fields below, or change the project type under General.</p>}
        {f.category === '3d-printing' && <PrintLog log={module.printLog || []} onChange={(printLog) => setModule({ ...module, printLog })} />}
        <CustomFields value={module.custom || {}} onChange={(custom) => setModule({ ...module, custom })} />
      </>}

      {tab === 'files' && <>
        <h3 style={{ marginBottom: 6 }}>Keep earlier versions of files</h3>
        <p className="small muted" style={{ marginTop: 0 }}>Mothball stores a copy whenever a file changes, so you can compare versions and roll back — even without Git. Identical content is only stored once.</p>
        {pol && <>
          <label className="row small" style={{ marginBottom: 6 }}><input type="checkbox" checked={pol.text} onChange={(e) => setPol({ ...pol, text: e.target.checked })} /> Code, config and text files up to <input className="input" type="number" min={0.1} step={0.5} style={{ width: 70, padding: '2px 6px' }} value={pol.textMaxMb} onChange={(e) => setPol({ ...pol, textMaxMb: Number(e.target.value) })} /> MB</label>
          {pol.keyExts.length > 0 && <label className="row small" style={{ marginBottom: 6 }}><input type="checkbox" checked={pol.keyFiles} onChange={(e) => setPol({ ...pol, keyFiles: e.target.checked })} /> Project files ({pol.keyExts.map((x: string) => '.' + x).join(', ')}) up to <input className="input" type="number" min={1} step={10} style={{ width: 80, padding: '2px 6px' }} value={pol.keyMaxMb} onChange={(e) => setPol({ ...pol, keyMaxMb: Number(e.target.value) })} /> MB</label>}
          <p className="small dim">Any file can also be saved by hand from its History panel.</p>
        </>}
        <div style={{ margin: '10px 0' }}><Retention projectId={p.id} onDone={() => stats.reload()} /></div>
        <h3 style={{ margin: '16px 0 6px' }}>Folders and file types to skip</h3>
        <p className="small dim" style={{ marginTop: 0 }}>One per line: <code>Library/</code> (folder anywhere), <code>/demos/</code> (that folder at the top only), <code>Assets/Generated/</code> (folder path), <code>*.tmp</code>, or a file name. .git and node_modules are always skipped.</p>
        <textarea className="input code" rows={5} value={f.ignoreGlobs} onChange={(e) => setF({ ...f, ignoreGlobs: e.target.value })} />
      </>}
    </Modal>
  );
}

function PrintLog({ log, onChange }: { log: any[]; onChange: (l: any[]) => void }) {
  const [draft, setDraft] = useState({ date: localToday(), result: 'success', notes: '', grams: '', time: '' });
  return (
    <div style={{ marginTop: 6 }}>
      <h3 style={{ marginBottom: 8 }}>Print history</h3>
      {log.length === 0 && <div className="small dim" style={{ marginBottom: 8 }}>No prints logged yet.</div>}
      {log.map((e, i) => (
        <div key={i} className="row small" style={{ padding: '3px 0' }}>
          {e.result === 'success' ? <LuCircleCheck color="var(--ok)" /> : <LuCircleX color="var(--bad)" />}
          <span style={{ width: 90 }} title={e.source === 'printer' ? 'Logged automatically from the printer' : undefined}>{logDate(e)}</span><span className="grow">{e.notes}</span>{(e.filaments || []).map((f: any, j: number) => <SpoolChip key={j} s={f} />)}<span className="dim">{[looseGrams(e) && `${looseGrams(e)} g`, e.time].filter(Boolean).join(' · ')}</span>
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

/** Fills empty print fields from what a .3mf in the project says (only empty fields; nothing is saved until Save). */
/** Fills addon details from a WoW .toc file. */
function FillFromToc({ projectId, module, onFill }: { projectId: string; module: Record<string, any>; onFill: (m: Record<string, any>) => void }) {
  const files = useLoad(() => call<any[]>('files.list', { projectId, exts: ['toc'], limit: 50 }), [projectId]);
  const [pick, setPick] = useState('');
  const [msg, setMsg] = useState('');
  const act = useAction();
  if (!files.data?.length) return null;
  const fill = async () => {
    const d: any = await act(() => call('symbols.toc', { fileId: pick || files.data![0].id }));
    if (!d) return;
    const fl = d.fields || {};
    const clean = (x?: string) => (x || '').replace(/\|c[0-9a-fA-F]{8}|\|r/g, '');
    const found: Record<string, string> = {
      game: d.gameVersions.length ? `World of Warcraft (${d.gameVersions.map((v: string) => (Number(v.split('.')[0]) >= 11 ? 'Retail' : 'Classic')).filter((x: string, i: number, a: string[]) => a.indexOf(x) === i).join(', ')})` : '',
      interface: fl.Interface || '',
      addonVersion: clean(fl.Version),
      savedVariables: [fl.SavedVariables, fl.SavedVariablesPerCharacter].filter(Boolean).join(', '),
      dependencies: [fl.RequiredDeps || fl.Dependencies, fl.OptionalDeps && `Optional: ${fl.OptionalDeps}`, fl['X-Embeds'] && `Embeds: ${fl['X-Embeds']}`].filter(Boolean).join('\n'),
      pageUrl: fl['X-Website'] || fl['X-Curse-Project-ID'] && `https://www.curseforge.com/projects/${fl['X-Curse-Project-ID']}` || '',
    };
    const next = { ...module };
    const filled: string[] = [];
    for (const [k, v] of Object.entries(found)) if (v && !String(next[k] || '').trim()) { next[k] = v; filled.push(k); }
    onFill(next);
    setMsg(filled.length ? `Filled ${filled.length} empty field(s). Check them, then Save.` : 'Nothing new: every field the .toc has is already filled in.');
  };
  return (
    <div className="alert info" style={{ marginBottom: 12 }}>
      <div className="grow">
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <span>Fill empty fields from</span>
          <select className="input" style={{ width: 'auto', maxWidth: 320 }} value={pick || files.data[0].id} onChange={(e) => setPick(e.target.value)}>{files.data.map((x) => <option key={x.id} value={x.id}>{x.path}</option>)}</select>
          <button className="btn sm" onClick={fill}>Fill</button>
        </div>
        {msg && <div className="small" style={{ marginTop: 4 }}>{msg}</div>}
      </div>
    </div>
  );
}

function FillFrom3mf({ projectId, module, onFill }: { projectId: string; module: Record<string, any>; onFill: (m: Record<string, any>) => void }) {
  const files = useLoad(() => call<any[]>('files.list', { projectId, exts: ['3mf'], limit: 200 }), [projectId]);
  const [pick, setPick] = useState('');
  const [msg, setMsg] = useState('');
  const act = useAction();
  if (!files.data?.length) return null;
  const fill = async () => {
    const id = pick || files.data![0].id;
    const d: any = await act(() => call('files.threemfInfo', { id }));
    if (!d) return;
    const s = d.settings || {};
    const fil: any[] = s.filaments || [];
    const plates: any[] = d.plates || [];
    const secs = plates.reduce((a, p) => a + (p.printSeconds || 0), 0);
    const grams = plates.reduce((a, p) => a + (p.weightG || 0), 0);
    const found: Record<string, string> = {
      material: [...new Set(fil.map((x) => x.type).filter(Boolean))].join(', '),
      filamentColor: fil.map((x) => x.color).filter(Boolean).join(', '),
      filamentGrams: grams ? grams.toFixed(1) : '',
      printTime: secs ? `${Math.floor(secs / 3600)}h ${Math.round((secs % 3600) / 60)}m` : '',
      printer: s.printerModel || s.printer || '',
      nozzle: s.nozzle ? `${s.nozzle} mm` : '',
      layerHeight: s.layerHeight ? `${s.layerHeight} mm` : '',
      slicer: d.application || '',
      slicerProfile: s.process || '',
    };
    const next = { ...module };
    const filled: string[] = [];
    for (const [k, v] of Object.entries(found)) if (v && !String(next[k] || '').trim()) { next[k] = v; filled.push(k); }
    onFill(next);
    setMsg(filled.length ? `Filled ${filled.length} empty field(s). Check them, then Save.` : 'Nothing new: every field the .3mf has is already filled in.');
  };
  return (
    <div className="alert info" style={{ marginBottom: 12 }}>
      <div className="grow">
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <span>Fill empty fields from</span>
          <select className="input" style={{ width: 'auto', maxWidth: 320 }} value={pick || files.data[0].id} onChange={(e) => setPick(e.target.value)}>{files.data.map((x) => <option key={x.id} value={x.id}>{x.path}</option>)}</select>
          <button className="btn sm" onClick={fill}>Fill</button>
        </div>
        {msg && <div className="small" style={{ marginTop: 4 }}>{msg}</div>}
      </div>
    </div>
  );
}

export function CustomFields({ value, onChange, compact }: { value: Record<string, string>; onChange: (v: Record<string, string>) => void; compact?: boolean }) {
  const [k, setK] = useState('');
  const [v, setV] = useState('');
  return (
    <div style={{ marginTop: compact ? 0 : 14 }}>
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

