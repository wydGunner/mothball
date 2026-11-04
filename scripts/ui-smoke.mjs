// End-to-end UI test: runs the real UI against the real core (via the dev
// server) in headless Chromium and walks through the critical workflows.
//
//   cargo build --manifest-path crates/vault-core/Cargo.toml --bin vault-devserver
//   npm run build:ui
//   node scripts/ui-smoke.mjs [--shots <dir>]
//
// Needs git on PATH. Exits non-zero on the first failed step.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, appendFileSync, renameSync, existsSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import { deflateSync } from 'node:zlib';
import { createServer } from 'node:net';
import { createSocket } from 'node:dgram';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const arg = (k) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : undefined; };
const shots = arg('--shots');
if (shots) mkdirSync(shots, { recursive: true });

function loadPlaywright() {
  const paths = [join(root, 'node_modules'), ...(process.env.NODE_PATH || '').split(/[:;]/).filter(Boolean)];
  for (const p of paths) { try { return createRequire(join(p, 'x.js'))('playwright'); } catch { /* next */ } }
  throw new Error('playwright not found (npm install)');
}
const { chromium } = loadPlaywright();

// ---- fixtures
const tmp = mkdtempSync(join(tmpdir(), 'pv-smoke-'));
const omen = join(tmp, 'OMEN');
mkdirSync(join(omen, 'scripts/mp'), { recursive: true });
mkdirSync(join(omen, 'configs'), { recursive: true });
writeFileSync(join(omen, 'scripts/mp/weapon_tuner.gsc'), 'init()\n{\n    level.instashoot = getDvarInt("omen_instashoot");\n}\n');
writeFileSync(join(omen, 'scripts/mp/_util.gsc'), 'util() {}\n');
writeFileSync(join(omen, 'configs/weapons.json'), '{"instashoot": true}\n');
mkdirSync(join(omen, 'menus'), { recursive: true });
writeFileSync(join(omen, 'scripts/mp/createserver.menu'), 'menu "createserver" { itemDef server }\n');
writeFileSync(join(omen, 'menus/createserver.menu'), 'menu "createserver" { itemDef lobby }\n');
const git = (...a) => execFileSync('git', ['-C', omen, '-c', 'user.name=Dev', '-c', 'user.email=dev@example.com', ...a], { stdio: 'pipe' });
git('init', '-q'); git('add', '-A'); git('commit', '-qm', 'Initial OMEN import');
appendFileSync(join(omen, 'scripts/mp/weapon_tuner.gsc'), '\nonPlayerConnect() {}\n'); git('commit', '-qam', 'Add player connect hook');
// a small 3D-printing project: an STL cube and two bright pictures
const deskCat = join(tmp, 'Desk Cat');
mkdirSync(deskCat);
{
  const f = (n, a, b, c) => `facet normal ${n}\n outer loop\n${[a, b, c].map((v) => `  vertex ${v.join(' ')}`).join('\n')}\n endloop\nendfacet\n`;
  const V = [[0, 0, 0], [20, 0, 0], [20, 30, 0], [0, 30, 0], [0, 0, 10], [20, 0, 10], [20, 30, 10], [0, 30, 10]];
  const tris = [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
  writeFileSync(join(deskCat, 'desk cat.stl'), 'solid cat\n' + tris.map(([a, b, c]) => f('0 0 0', V[a], V[b], V[c])).join('') + 'endsolid cat\n');
  const crcT = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = 0xffffffff; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
  const png = (w, h, px) => {
    const raw = Buffer.alloc((w * 3 + 1) * h);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) raw.set(px(x, y), y * (w * 3 + 1) + 1 + x * 3);
    const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
    return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
  };
  writeFileSync(join(deskCat, 'Green and yellow cat.png'), png(320, 240, (x, y) => [90 + (x % 120), 200, 60 + (y % 100)]));
  writeFileSync(join(deskCat, 'cat on the desk.png'), png(320, 240, (x, y) => [230, 180 + (x % 60), 80 + (y % 120)]));
}
const backups = join(tmp, 'backups'); mkdirSync(backups);
const bare = join(tmp, 'remote.git'); mkdirSync(bare);
execFileSync('git', ['-C', bare, 'init', '--bare', '-q']);
git('remote', 'add', 'origin', bare);

// ---- server
const exe = join(root, 'crates/vault-core/target/debug', process.platform === 'win32' ? 'vault-devserver.exe' : 'vault-devserver');
const port = 14000 + Math.floor(Math.random() * 1000);
const srv = spawn(exe, [join(tmp, 'data'), join(root, 'ui/dist'), String(port)], { stdio: 'inherit' });
await new Promise((r) => setTimeout(r, 700));
const base = `http://127.0.0.1:${port}`;

const browser = await chromium.launch({ args: ['--use-gl=swiftshader', '--enable-webgl', '--ignore-gpu-blocklist'] }); // WebGL for the 3D preview
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await context.grantPermissions(['clipboard-read', 'clipboard-write']);
const page = await context.newPage();
if (process.env.PV_CPU_SLOW) { const cdp = await context.newCDPSession(page); await cdp.send('Emulation.setCPUThrottlingRate', { rate: Number(process.env.PV_CPU_SLOW) }); }
const themeArg = arg('--theme'); // e.g. sunset, twilight — for screenshots
if (themeArg) {
  const T = { sunset: ['#ff7a1a', '#ffb224', '#f0384b'], twilight: ['#8b5cf6', '#3b82f6', '#ec4899'], ocean: ['#38bdf8', '#6366f1', '#22d3ee'] }[themeArg];
  if (T) await page.addInitScript((t) => localStorage.setItem('pv.theme', JSON.stringify({ id: t[3], name: t[3], a1: t[0], a2: t[1], a3: t[2] })), [...T, themeArg]);
}
const errors = [];
page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });
let n = 0;
const step = async (name, fn) => {
  process.stdout.write(`• ${name} … `);
  try { await fn(); console.log('ok'); if (shots) await page.screenshot({ path: join(shots, `${String(++n).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`) }); }
  catch (e) { console.log('FAILED'); console.error(e); console.error(errors.join('\n'));
    // CI artifacts are not always reachable, so put what was on screen into the log too
    try { console.error('--- page text at failure ---\n' + (await page.locator('main, body').first().innerText({ timeout: 2000 })).slice(0, 4000)); } catch {}
    if (shots) await page.screenshot({ path: join(shots, 'FAILED.png') }); await browser.close(); srv.kill(); process.exit(1); }
};
const api = (cmd, args = {}) => page.evaluate(async ([c, a]) => (await (await fetch('/api/' + c, { method: 'POST', body: JSON.stringify(a) })).json()), [cmd, args]);
const toastGone = async () => { while (await page.locator('.toast button').count()) await page.locator('.toast button').first().click(); };

await page.goto(base);
// most steps were written for one project at a time; the project-tabs step turns tabs on
await page.evaluate(() => localStorage.setItem('pv.maxTabs', '1'));

await step('First run: welcome screen', async () => {
  await page.getByRole('button', { name: /Add a folder I already have/ }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-welcome.png') });
});

await step('Create OMEN v2 linked project from the welcome screen', async () => {
  page.once('dialog', (d) => d.accept(omen));
  await page.getByRole('button', { name: /Add a folder I already have/ }).click();
  // the folder has GSC scripts, so the type is suggested
  await page.getByText(/Suggested IW4X \/ MW2 mod because the folder has GSC scripts/).waitFor();
  await page.locator('.wl-type.on').filter({ hasText: 'IW4X / MW2 mod' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-welcome-type.png') });
  await page.locator('.wl-form input.input').fill('OMEN v2');
  await page.locator('.wl-type').filter({ hasText: 'Software' }).click();
  await page.getByRole('button', { name: 'Add project' }).click();
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).waitFor();
  await page.locator('.chip.on').filter({ hasText: 'All files' }).waitFor(); // never opens pre-filtered
  await page.locator('.firsttips').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-first-tips.png') });
  await page.locator('.firsttips button').click();
  await page.locator('.firsttips').waitFor({ state: 'detached' });
  const p = (await api('projects.list')).data.find((x) => x.name === 'OMEN v2');
  if (p.category !== 'software') throw new Error('type not saved: ' + p.category);
});

await step('Browse folder tree', async () => {
  await page.locator('.tnode').filter({ hasText: 'configs' }).click();
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).waitFor({ state: 'detached' });
  await page.locator('.filerow').filter({ hasText: 'weapons.json' }).waitFor();
  await page.locator('.tnode').filter({ hasText: '(all files)' }).click();
});

await step('Document weapon_tuner.gsc', async () => {
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).click();
  // an undocumented file shows one line plus "add" chips, not a wall of empty fields
  if (await page.locator('.doc textarea').count()) throw new Error('empty docs should not open editors');
  await page.getByPlaceholder(/One line, e\.g\./).fill('Handles weapon-specific tuning and behavior');
  await page.evaluate(() => { // diagnostics for a CI-only failure: what did the pointer hit, and did the chip move?
    const log = (window.__ev = []);
    const at = (e) => { const c = [...document.querySelectorAll('.addrow .chip')].find((x) => x.textContent.includes('Notes')); const r = c?.getBoundingClientRect(); log.push(`${e.type} ${e.clientX},${e.clientY} on ${e.target.className || e.target.tagName} chip@${r ? Math.round(r.x) + ',' + Math.round(r.y) : 'gone'}`); };
    for (const t of ['pointerdown', 'mousedown', 'mouseup', 'click']) window.addEventListener(t, at, true);
    new MutationObserver(() => { if (!document.querySelector('.doc')) log.push('doc panel gone'); }).observe(document.body, { childList: true, subtree: true });
  });
  await page.locator('.addrow .chip').filter({ hasText: 'Notes' }).click();
  await page.locator('textarea.code').waitFor({ timeout: 8000 }).catch(async () => { throw new Error('notes editor did not open; events: ' + JSON.stringify(await page.evaluate(() => window.__ev))); });
  await page.locator('textarea.code').fill('- Contains instashoot configuration\n- Depends on [[file:configs/weapons.json]]\n- Requires dedicated server testing');
  await page.locator('.addrow .chip').filter({ hasText: 'Warning' }).click();
  await page.getByPlaceholder('e.g. Never edit while the server is running').fill('Requires dedicated server testing before release');
  await page.locator('.savestate > span:visible').filter({ hasText: /^Saved$/ }).waitFor({ timeout: 5000 });
  await page.getByRole('button', { name: 'Done' }).click();
  await page.locator('.md').getByText('Contains instashoot configuration').waitFor();
});

await step('Link script to related file, note and bug', async () => {
  const pid = (await api('projects.list')).data[0].id;
  const note = await api('notes.create', { projectId: pid, title: 'Weapon Systems', body: 'See [[file:scripts/mp/weapon_tuner.gsc]]' });
  const bug = await api('tasks.create', { projectId: pid, title: 'Instashoot desync on dedicated', kind: 'bug' });
  if (!note.ok || !bug.ok) throw new Error('setup failed');
  for (const [tab, name] of [['File', '_util.gsc'], ['Note', 'Weapon Systems'], ['Task / bug', 'Instashoot desync']]) {
    await page.locator('.addrow .chip').filter({ hasText: /link/i }).click();
    await page.locator('.modal .seg button').filter({ hasText: tab }).click();
    await page.locator('.modal .filerow').filter({ hasText: name }).first().click();
    await page.locator('.doc').getByText(name).first().waitFor();
  }
});

await step('Internal link opens the linked file', async () => {
  await page.locator('.md a').filter({ hasText: 'configs/weapons.json' }).click();
  await page.locator('.doc h2').filter({ hasText: 'weapons.json' }).waitFor();
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).click();
});

await step('Detect external modification', async () => {
  appendFileSync(join(omen, 'scripts/mp/weapon_tuner.gsc'), '\n// tweak\n');
  const f = join(omen, 'scripts/mp/weapon_tuner.gsc');
  execFileSync(process.execPath, ['-e', `require('fs').utimesSync(${JSON.stringify(f)}, new Date(), new Date(Date.now()+5000))`]);
  await toastGone();
  await page.getByRole('button', { name: 'Rescan' }).click();
  await page.locator('.toast').filter({ hasText: '1 modified' }).waitFor();
  await page.getByText("content changed after this documentation was last written").waitFor();
  // the outside change shows what changed, like an edit made in Mothball
  await page.getByRole('button', { name: 'Activity', exact: true }).click();
  const act = page.locator('.doc .row').filter({ hasText: 'Content changed outside Mothball' }).first();
  await act.getByRole('button', { name: 'Show changes' }).click(); // collapsed until asked
  await page.locator('.doc .tl-diff').getByText('// tweak').first().waitFor();
  await page.locator('.doc .tl-diff .tl-add').filter({ hasText: '+2' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-outside-change-diff.png') });
});

await step('Outside edit to a .menu file shows its diff in the Timeline, and vN in the list', async () => {
  // .menu used to be filed as "other": nothing was kept, so it only said "changed on disk"
  const f = join(omen, 'scripts/mp/createserver.menu');
  writeFileSync(f, 'menu "createserver" { itemDef server }\nitemDef password\n');
  execFileSync(process.execPath, ['-e', `require('fs').utimesSync(${JSON.stringify(f)}, new Date(), new Date(Date.now()+5000))`]);
  await toastGone();
  await page.getByRole('button', { name: 'Rescan' }).click();
  await page.locator('.toast').filter({ hasText: '1 modified' }).waitFor();
  const row = page.locator('.filerow').filter({ hasText: 'createserver.menu' }).filter({ has: page.locator('.badge', { hasText: /^v2$/ }) });
  await row.waitFor();
  await row.locator('.badge.warn').filter({ hasText: 'changed' }).waitFor(); // both, side by side
  await row.click();
  await page.getByRole('button', { name: 'Timeline', exact: true }).click();
  // the diff is in the Timeline but collapsed until asked
  await page.locator('.tline').getByRole('button', { name: 'Show changes' }).first().waitFor();
  if (await page.locator('.tline .tl-diff').count()) throw new Error('Timeline diff should start collapsed');
  await page.locator('.tline').getByRole('button', { name: 'Show changes' }).first().click();
  await page.locator('.tline .tl-diff').getByText('itemDef password').first().waitFor();
  await page.locator('.tline .tl-diff .tl-add').filter({ hasText: '+1' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-menu-timeline-diff.png') });
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).click();
});

await step('Compare file versions (no Git needed)', async () => {
  await page.getByRole('button', { name: /^Backups & versions/ }).click();
  await page.locator('.verrow').nth(1).getByRole('button', { name: /vs now/ }).click();
  // Monaco's diff view when monaco-editor is installed (CI), the built-in one otherwise
  const added = '.modal .dl.add, .modal .half.add, .modal .monaco-diff-editor .line-insert';
  await page.locator(added).first().waitFor();
  await page.locator('.modal').getByText('// tweak').first().waitFor();
  const monaco = (await page.locator('.modal .monaco-diff-editor').count()) > 0;
  await page.locator('.modal').getByRole('button', { name: /Unified/ }).click();
  if (monaco) { await page.locator('.modal .monaco-diff-editor .line-insert').first().waitFor(); await page.locator('.modal .monaco-diff-editor').getByText('// tweak').first().waitFor(); }
  else await page.locator('.modal .dl.add .tx').filter({ hasText: '// tweak' }).waitFor();
  await page.locator('.modal').getByRole('button', { name: /Side by side/ }).click();
  if (shots) await page.screenshot({ path: join(shots, 'x-diff.png') });
  await page.keyboard.press('Escape');
});

await step('Git history for the file', async () => {
  await page.getByRole('button', { name: 'Git commits' }).click();
  await page.getByText('Add player connect hook').waitFor();
  await page.getByText('Add player connect hook').click();
  await page.locator('.diff .add').first().waitFor();
});

await step('Rename + move keeps documentation', async () => {
  mkdirSync(join(omen, 'scripts/weapons'), { recursive: true });
  renameSync(join(omen, 'scripts/mp/weapon_tuner.gsc'), join(omen, 'scripts/weapons/tuner.gsc'));
  await toastGone();
  await page.getByRole('button', { name: 'Rescan' }).click();
  await page.locator('.toast').filter({ hasText: '1 moved/renamed' }).waitFor();
  await page.locator('.filerow').filter({ hasText: 'tuner.gsc' }).filter({ hasText: 'Handles weapon-specific' }).click();
  await page.locator('.doc .mono').filter({ hasText: 'scripts/weapons/tuner.gsc' }).first().waitFor();
});

await step('Project notes knowledge base', async () => {
  await page.getByRole('tab', { name: /Notes/ }).click();
  await page.locator('.tnode').filter({ hasText: 'Weapon Systems' }).click();
  await page.locator('textarea.body').fill('Architecture of weapon systems.\n\n[[file:scripts/weapons/tuner.gsc]]');
  await page.locator('.savestate > span:visible').filter({ hasText: /^Saved$/ }).waitFor({ timeout: 5000 });
  await page.getByRole('button', { name: 'Note', exact: true }).click();
  await page.locator('input.title[value="Untitled"]').waitFor();
  await page.locator('input.title').fill('Server Setup');
  await page.locator('.savestate > span:visible').filter({ hasText: /^Saved$/ }).waitFor({ timeout: 5000 });
  await page.locator('.tnode').filter({ hasText: 'Server Setup' }).waitFor();
  await page.locator('.tnode').filter({ hasText: 'Weapon Systems' }).waitFor();
});

await step('Tasks board, bug details', async () => {
  await page.getByRole('tab', { name: /Tasks/ }).click();
  await page.getByRole('button', { name: /Board/ }).click();
  await page.locator('.tcard').filter({ hasText: 'Instashoot desync' }).click();
  await page.getByPlaceholder('1. Start dedicated server\n2. …').fill('1. Start dedicated server\n2. Fire cheytac');
  await page.locator('.modal select').first().selectOption('in_progress');
  await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
  await page.locator('.kcol').filter({ hasText: 'In progress' }).locator('.tcard').filter({ hasText: 'Instashoot' }).waitFor();
});

await step('Timeline with journal entry', async () => {
  await page.getByRole('tab', { name: /Timeline/ }).click();
  await page.getByPlaceholder('What did you do / decide / learn?').fill('Decided to move weapon scripts into scripts/weapons');
  await page.getByRole('button', { name: 'Add to timeline' }).click();
  await page.locator('.tlitem.manual').filter({ hasText: 'Decided to move' }).waitFor();
  await page.locator('.tlitem').filter({ hasText: 'File changes detected' }).first().waitFor();
});

await step('Edit a file in the built-in editor', async () => {
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.locator('.chip').filter({ hasText: 'All files' }).click();
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).click();
  await page.getByRole('button', { name: 'Edit file' }).click();
  const ed = page.locator('.pv-editor textarea').first(); // Monaco and the stand-in both take keyboard input via a textarea
  await ed.waitFor();
  await ed.focus();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('// edited in Mothball');
  await page.getByText('Unsaved — Ctrl+S to save').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-editor.png') });
  const before = readFileSync(join(omen, 'scripts/mp/_util.gsc'), 'utf8');
  // default save mode asks: Ctrl+S opens Smart Save; Cancel changes nothing
  await page.keyboard.press('Control+S');
  const dlg = page.locator('.modal').filter({ hasText: 'What changed in this version?' });
  await dlg.waitFor();
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  await dlg.waitFor({ state: 'detached' });
  if (readFileSync(join(omen, 'scripts/mp/_util.gsc'), 'utf8') !== before) throw new Error('Cancel wrote the file');
  await page.getByText('Unsaved — Ctrl+S to save').waitFor();
  // neutral name with no description; descriptive once described
  await page.waitForFunction(() => document.activeElement?.closest('.pv-editor') != null); // focus returns to the editor
  await page.keyboard.press('Control+S');
  await dlg.waitFor();
  await dlg.locator('input.mono').first().evaluate((el) => new Promise((r) => { const t0 = Date.now(); const w = () => (el.value || Date.now() - t0 > 3000 ? r() : setTimeout(w, 50)); w(); }));
  const neutral = await dlg.locator('input.mono').first().inputValue();
  if (!/^_util_v\d+_\d{4}-\d{2}-\d{2}\.gsc$/.test(neutral)) throw new Error('unexpected neutral name: ' + neutral);
  await dlg.locator('textarea').fill('Instashoot fix: moved timing check before animation');
  await page.waitForFunction(() => /instashoot/i.test(document.querySelector('.modal input.mono')?.value || ''));
  if (shots) await page.screenshot({ path: join(shots, 'x-smart-save.png') });
  await dlg.getByRole('button', { name: 'Save with backup', exact: true }).click();
  await page.getByText(/Saved with backup/).first().waitFor();
  const disk = readFileSync(join(omen, 'scripts/mp/_util.gsc'), 'utf8');
  if (!disk.includes('// edited in Mothball')) throw new Error('editor save did not reach disk: ' + disk);
  const fid = (await api('search', { query: '_util', types: ['file'] })).data[0].id;
  const vs = (await api('versions.list', { fileId: fid })).data;
  const pre = vs.find((v) => v.reason === 'pre-save');
  if (!pre || !/instashoot.*_before\.gsc$/.test(pre.name) || !pre.note.includes('timing check')) throw new Error('pre-save backup missing: ' + JSON.stringify(vs));
  // external change while open -> conflict prompt, nothing overwritten silently
  writeFileSync(join(omen, 'scripts/mp/_util.gsc'), disk + '\n// external\n');
  await page.locator('.pv-editor textarea').first().focus();
  await page.keyboard.type(' again');
  await page.keyboard.press('Control+S');
  await dlg.waitFor();
  await dlg.getByRole('button', { name: 'Save normally' }).click();
  await page.getByText('File changed on disk').waitFor();
  await page.locator('.modal').getByRole('button', { name: 'Cancel' }).click();
  if (!readFileSync(join(omen, 'scripts/mp/_util.gsc'), 'utf8').includes('// external')) throw new Error('external change was overwritten');
  // closing a tab with unsaved changes asks first
  const tab = page.locator('.edtab').filter({ hasText: '_util.gsc' });
  let asked = '';
  page.once('dialog', (d) => { asked = d.message(); d.accept(); });
  await tab.locator('.edtab-x').click();
  await tab.waitFor({ state: 'detached' });
  if (!/Discard unsaved changes/.test(asked)) throw new Error('no discard prompt: ' + asked);
  await page.getByRole('button', { name: /^Backups & versions/ }).waitFor();
});

await step('Editor tabs keep unsaved edits while switching files', async () => {
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).click();
  await page.getByRole('button', { name: 'Edit file' }).click();
  await page.locator('.edtab.on').filter({ hasText: '_util.gsc' }).waitFor();
  await page.locator('.edslot:visible .pv-editor textarea').first().focus();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('// tab test');
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).locator('.dirty-dot').waitFor();
  // open a second file in its own tab
  const other = page.locator('.filerow').filter({ hasText: '.gsc' }).filter({ hasNotText: '_util.gsc' }).first();
  await other.click();
  await page.getByRole('button', { name: 'Edit file' }).click();
  await page.locator('.edtab').nth(1).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-editor-tabs.png') });
  // other project tab and back: the unsaved edit is still there
  await page.getByRole('tab', { name: /Overview/ }).click();
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).click();
  await page.waitForFunction(() => [...document.querySelectorAll('.edslot')].some((e) => e.style.display !== 'none' && ((e.textContent || '').includes('tab test') || [...e.querySelectorAll('textarea')].some((t) => t.value.includes('tab test')))));
  // project tabs: going to the dashboard keeps the project open (no question), and the edit survives
  await page.evaluate(() => localStorage.setItem('pv.maxTabs', '10'));
  let asked = false;
  const onDialog = (d) => { asked = true; d.dismiss(); };
  page.on('dialog', onDialog);
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.dash-bar').waitFor();
  if (asked) throw new Error('leaving for the dashboard asked about unsaved edits');
  const ptab = page.getByRole('tab', { name: /OMEN v2/ });
  await ptab.locator('.ptab-dirty').waitFor(); // unsaved marker on the project tab
  if (shots) await page.screenshot({ path: join(shots, 'x-project-tabs.png') });
  await ptab.click();
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).locator('.dirty-dot').waitFor();
  // closing the project tab asks; cancelling keeps it
  await ptab.getByRole('button', { name: 'Close OMEN v2' }).click();
  if (!asked) throw new Error('closing a project with unsaved edits did not ask');
  page.off('dialog', onDialog);
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).waitFor();
  // close both tabs (discarding)
  page.once('dialog', (d) => d.accept());
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).locator('.edtab-x').click();
  await page.locator('.edtab .edtab-x').first().click();
  await page.locator('.edtab').first().waitFor({ state: 'detached' }).catch(() => {});
  await page.evaluate(() => localStorage.setItem('pv.maxTabs', '1'));
});

await step('Right-click: highlight in the preview, edit it; the app menu replaces the browser menu', async () => {
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).click();
  const pre = page.locator('pre.preview');
  await pre.getByText('util()', { exact: false }).waitFor();
  // highlight "getDvarInt" in the preview, then right-click it
  await pre.evaluate((el) => {
    const t = el.firstChild; const i = t.textContent.indexOf('util()');
    const r = document.createRange(); r.setStart(t, i); r.setEnd(t, i + 4);
    const s = getSelection(); s.removeAllRanges(); s.addRange(r);
  });
  const box = await pre.boundingBox();
  await page.mouse.click(box.x + 20, box.y + 12, { button: 'right' });
  const item = page.locator('.ctx button').filter({ hasText: 'Edit “util” in the editor' });
  await item.waitFor();
  await page.locator('.ctx button').filter({ hasText: 'Copy' }).locator('.ctx-hint').filter({ hasText: 'Ctrl+C' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-preview-menu.png') });
  await item.click();
  // the editor opens with exactly that text selected (Monaco in CI, the stand-in here)
  await page.waitForFunction(() => {
    const slot = [...document.querySelectorAll('.edslot')].find((e) => e.style.display !== 'none');
    if (!slot) return false;
    const m = slot.querySelector('.monaco-editor');
    if (m) return !!m.querySelector('.selected-text') && (window.getSelection()?.toString() === '' || true);
    const ta = slot.querySelector('textarea');
    return ta && ta.value.slice(ta.selectionStart, ta.selectionEnd) === 'util';
  });
  if (shots) await page.screenshot({ path: join(shots, 'x-preview-edit-selected.png') });
  await page.locator('.edtab').filter({ hasText: '_util.gsc' }).locator('.edtab-x').click();
  await page.locator('.edtab').first().waitFor({ state: 'detached' }).catch(() => {});
  // the search box gets the app's text menu, not the browser's
  await page.getByPlaceholder('Search files…').click({ button: 'right' });
  for (const l of ['Cut', 'Copy', 'Paste', 'Select all']) await page.locator('.ctx button').filter({ hasText: l }).first().waitFor();
  await page.keyboard.press('Escape');
  await page.locator('.ctx').waitFor({ state: 'detached' });
  // empty space: the browser menu (Back, Refresh, Print…) is suppressed
  const prevented = await page.evaluate(() => { const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }); document.querySelector('.side, aside, body').dispatchEvent(ev); return ev.defaultPrevented; });
  if (!prevented) throw new Error('the browser menu was not suppressed');
  if (await page.locator('.ctx').count()) throw new Error('no menu expected on empty space');
});

await step('Timeline: expand an edit to see what changed', async () => {
  await page.getByRole('tab', { name: /Timeline/ }).click();
  const item = page.locator('.tlitem').filter({ hasText: 'Edited scripts/mp/_util.gsc' }).first();
  await item.locator('.tl-toggle').click();
  await item.locator('.tl-diff .dl.add').filter({ hasText: 'edited in Mothball' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-timeline-diff.png') });
  await item.getByRole('button', { name: 'Side by side…' }).click();
  await page.locator('.modal').filter({ hasText: 'Changes in' }).waitFor();
  await page.keyboard.press('Escape');
  await item.locator('.tl-toggle').click();
  await item.locator('.tl-detail').waitFor({ state: 'detached' });
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).click();
});

await step('Version history: search, compare, restore', async () => {
  await page.getByRole('button', { name: /^Backups & versions/ }).click();
  const row = page.locator('.verrow').filter({ hasText: '_before.gsc' });
  await row.waitFor();
  await page.getByPlaceholder('Search backups — name, description, tags…').fill('timing check');
  if (await page.locator('.verrow').count() !== 1) throw new Error('backup search should leave one row');
  await row.getByRole('button', { name: /vs now/ }).click();
  const cmp = page.locator('.modal').filter({ hasText: 'Changes in' });
  await cmp.getByText('// external').first().waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-compare.png') });
  await cmp.locator('.modal-head button, button[aria-label="Close"]').first().click().catch(() => page.keyboard.press('Escape'));
  await cmp.waitFor({ state: 'detached' });
  await row.getByRole('button', { name: /Restore/ }).click();
  const rd = page.locator('.modal').filter({ hasText: 'Restore this version?' });
  await rd.getByText('_util.gsc', { exact: false }).first().waitFor();
  await rd.getByText('Keep a backup of the current file first').waitFor();
  await rd.getByRole('button', { name: 'Restore' }).click();
  await rd.waitFor({ state: 'detached' });
  const now = readFileSync(join(omen, 'scripts/mp/_util.gsc'), 'utf8');
  if (now.includes('// edited in Mothball')) throw new Error('restore did not bring back the pre-save state: ' + now);
  const fid = (await api('search', { query: '_util', types: ['file'] })).data[0].id;
  const vs = (await api('versions.list', { fileId: fid })).data;
  if (!vs.some((v) => v.reason === 'before-restore' && v.sha256 && !v.isCurrent)) throw new Error('current file was not kept before restoring');
  // the per-file timeline labels sources
  await page.getByPlaceholder('Search backups — name, description, tags…').fill('');
  await page.locator('.seg button').filter({ hasText: 'Timeline' }).click();
  await page.locator('.srcbadge').filter({ hasText: 'Backup' }).first().waitFor();
  await page.locator('.srcbadge').filter({ hasText: 'Git commit' }).first().waitFor();
  // a restore is a Mothball action, not an outside change, and shows what it changed
  await page.getByRole('button', { name: 'Rescan' }).click();
  await page.locator('.seg button').filter({ hasText: 'Activity' }).click();
  const restoredRow = page.locator('.doc .row').filter({ hasText: /^restored-version/ }).first();
  await restoredRow.waitFor();
  const latest = await page.locator('.doc .row .pill').first().textContent();
  if (latest !== 'restored-version') throw new Error('restore was followed by: ' + latest);
  await restoredRow.getByRole('button', { name: 'Show changes' }).click();
  await page.locator('.doc .tl-diff .tl-del').first().waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-restore-diff.png') });
});

await step('Global search finds backups (backup actions only)', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('before animation');
  const it = page.locator('.palette .item.backupitem').first();
  await it.waitFor();
  await it.getByText('backup copy').waitFor();
  await it.click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Compare with current file' }).waitFor();
  await page.locator('.ctx button').filter({ hasText: 'Restore this backup…' }).waitFor();
  for (const bad of ['Delete file from disk…', 'Rename…', 'Edit in Mothball']) if (await page.locator('.ctx button').filter({ hasText: bad }).count()) throw new Error('live-file action on a backup: ' + bad);
  await page.keyboard.press('Escape');
  await page.locator('.ctx').waitFor({ state: 'detached' });
  await page.keyboard.press('Enter'); // opens the file's history with the backup highlighted
  await page.locator('.verrow.flashv, .verrow').filter({ hasText: '_before.gsc' }).first().waitFor();
});

await step('Project snapshots: create, preview, restore', async () => {
  await page.getByRole('tab', { name: /Backups/ }).click();
  await page.locator('[data-testid=backup-row]').first().waitFor();
  await page.getByRole('button', { name: 'Create snapshot…' }).click();
  await page.locator('.modal').getByPlaceholder('e.g. Working before animation rewrite').fill('Known good before tuning');
  await page.locator('.modal').getByRole('button', { name: 'Create snapshot' }).click();
  await page.locator('[data-testid=snapshot-row]').filter({ hasText: 'Known good before tuning' }).waitFor();
  const good = readFileSync(join(omen, 'configs/weapons.json'), 'utf8');
  writeFileSync(join(omen, 'configs/weapons.json'), '{"instashoot": false}\n');
  writeFileSync(join(omen, 'configs/new_after_snapshot.cfg'), 'keep me\n');
  await page.locator('[data-testid=snapshot-row]').filter({ hasText: 'Known good before tuning' }).getByRole('button', { name: /Restore/ }).click();
  const m = page.locator('.modal').filter({ hasText: 'Restore snapshot' });
  await m.getByText('will be replaced with their snapshot content').waitFor();
  await m.getByText(/added since the snapshot are/).waitFor();
  await m.locator('li').filter({ hasText: 'added since the snapshot' }).getByText('1', { exact: true }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-snapshot-restore.png') });
  await m.getByRole('button', { name: /Restore \d+ file/ }).click();
  await page.locator('.modal').filter({ hasText: 'Snapshot restored' }).getByRole('button', { name: 'Done' }).click();
  if (readFileSync(join(omen, 'configs/weapons.json'), 'utf8') !== good) throw new Error('snapshot restore failed');
  if (!existsSync(join(omen, 'configs/new_after_snapshot.cfg'))) throw new Error('snapshot restore deleted a newer file');
  await page.locator('[data-testid=snapshot-row]').filter({ hasText: 'Before restoring' }).waitFor();
});

await step('Import with preview, then Undo import', async () => {
  const src = join(tmp, 'incoming'); mkdirSync(src, { recursive: true });
  writeFileSync(join(src, 'loot_table.json'), '{"a":1}\n');
  writeFileSync(join(src, 'weapons.json'), 'conflicting name\n');
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.locator('.tnode').filter({ hasText: 'configs' }).first().click();
  page.once('dialog', (d) => d.accept([join(src, 'loot_table.json'), join(src, 'weapons.json')].join(';')));
  await page.getByRole('button', { name: 'Import', exact: true }).click();
  const m = page.locator('.modal').filter({ hasText: 'Import files' });
  await m.getByText('OMEN', { exact: false }).first().waitFor();
  await m.getByText(/weapons.*\.json/).first().waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-import.png') });
  await m.getByRole('button', { name: /Copy 2 files into/ }).click();
  await m.waitFor({ state: 'detached' });
  if (!existsSync(join(omen, 'configs/loot_table.json'))) throw new Error('import did not copy');
  if (readFileSync(join(omen, 'configs/weapons.json'), 'utf8').includes('conflicting')) throw new Error('import overwrote an existing file');
  if (!existsSync(join(src, 'loot_table.json'))) throw new Error('import moved the source');
  const renamed = readdirSync(join(omen, 'configs')).find((f) => /^weapons.+\.json$/.test(f) && f !== 'weapons.json');
  if (!renamed) throw new Error('conflict was not renamed');
  // edit one imported file after import: Undo must keep it
  writeFileSync(join(omen, 'configs', renamed), 'edited after import\n');
  await page.locator('.toast').getByRole('button', { name: 'Undo import' }).click();
  await page.getByText(/Removed 1 imported cop/).waitFor();
  if (existsSync(join(omen, 'configs/loot_table.json'))) throw new Error('undo import left the copy');
  if (!existsSync(join(omen, 'configs', renamed))) throw new Error('undo import removed a file changed after import');
  if (!existsSync(join(omen, 'configs/weapons.json'))) throw new Error('undo import removed an unrelated file');
  await toastGone();
  // Move instead of copy: the original leaves its folder; Undo puts it back
  page.once('dialog', (d) => d.accept(join(src, 'loot_table.json')));
  await page.getByRole('button', { name: 'Import', exact: true }).click();
  await m.locator('.opcard').filter({ hasText: 'Move into the project' }).click();
  await m.getByRole('button', { name: /Move 1 file into/ }).click();
  await m.waitFor({ state: 'detached' });
  if (existsSync(join(src, 'loot_table.json')) || !existsSync(join(omen, 'configs/loot_table.json'))) throw new Error('move did not move');
  await page.locator('.toast').getByRole('button', { name: 'Undo import' }).click();
  await page.getByText(/Moved 1 file\(s\) back/).waitFor();
  if (!existsSync(join(src, 'loot_table.json')) || existsSync(join(omen, 'configs/loot_table.json'))) throw new Error('undo did not move the file back');
  await toastGone();
});

await step('Import: edit files where they are (managed → your folder)', async () => {
  const mine = join(tmp, 'My Prints'); mkdirSync(mine, { recursive: true });
  writeFileSync(join(mine, 'Skeleton Glock.3mf'), 'glock');
  writeFileSync(join(mine, 'mag.stl'), 'mag');
  const pr = (await api('projects.create', { name: 'Prints In Place', category: '3d-printing' })).data;
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await page.goto(base + '/');
  await page.locator('.pcard').filter({ hasText: 'Prints In Place' }).click();
  await page.getByRole('tab', { name: /Files/ }).click();
  page.once('dialog', (d) => d.accept(join(mine, 'Skeleton Glock.3mf')));
  await page.getByRole('button', { name: 'Import', exact: true }).click();
  const m = page.locator('.modal').filter({ hasText: 'Import files' });
  await m.locator('.opcard').filter({ hasText: 'Edit them where they are' }).click();
  await m.getByText(mine).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-import-inplace.png') });
  await m.getByRole('button', { name: 'Use this folder' }).click();
  await m.waitFor({ state: 'detached' });
  await page.locator('.filerow').filter({ hasText: 'mag.stl' }).waitFor();
  const p2 = (await api('projects.get', { id: pr.id })).data;
  if (p2.mode !== 'linked' || !p2.rootPath.endsWith('My Prints')) throw new Error('project not switched to the folder: ' + JSON.stringify(p2));
  if (!existsSync(join(mine, 'Skeleton Glock.3mf'))) throw new Error('original file touched');
  await toastGone();
  await page.locator('.pcard, .sidebar a, nav a').first().waitFor().catch(() => {});
  await page.goto(base + '/');
  await page.locator('.pcard').filter({ hasText: 'OMEN v2' }).click();
  await page.getByRole('tab', { name: /Files/ }).click();
});

await step('Remove from Mothball keeps the file on disk', async () => {
  await page.locator('.tnode').filter({ hasText: '(all files)' }).click();
  await page.locator('.filerow').filter({ hasText: 'new_after_snapshot.cfg' }).click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Remove from Mothball…' }).click();
  const m = page.locator('.modal').last();
  await m.getByText('new_after_snapshot.cfg', { exact: false }).first().waitFor();
  await m.getByRole('button', { name: /Remove/ }).last().click();
  await page.locator('.filerow').filter({ hasText: 'new_after_snapshot.cfg' }).waitFor({ state: 'detached' });
  if (!existsSync(join(omen, 'configs/new_after_snapshot.cfg'))) throw new Error('remove deleted the file from disk');
  await page.keyboard.press('F5');
  await page.waitForTimeout(800);
  if (await page.locator('.filerow').filter({ hasText: 'new_after_snapshot.cfg' }).count()) throw new Error('removed file came back on rescan');
});

await step('Keyboard: context menu without the mouse', async () => {
  await page.locator('.filerow').filter({ hasText: 'weapons.json' }).first().click();
  await page.keyboard.press('Shift+F10');
  await page.locator('.ctx').waitFor();
  await page.keyboard.press('ArrowDown');
  // focus moves into the menu (allow a frame for it to render)
  await page.waitForFunction(() => document.activeElement?.closest('.ctx') != null, null, { timeout: 3000 }).catch(() => { throw new Error('menu items are not keyboard-focusable'); });
  await page.keyboard.press('Escape');
  await page.locator('.ctx').waitFor({ state: 'detached' });
});

await step('Git tab: commit and push', async () => {
  await page.getByRole('tab', { name: /Git/ }).click();
  await page.getByText('Initial OMEN import').waitFor();
  await page.getByText('Uncommitted changes').waitFor();
  const id = page.getByPlaceholder('Your name');
  if (await id.count()) { await id.fill('Dev'); await page.getByPlaceholder('you@example.com').fill('dev@example.com'); await page.getByRole('button', { name: 'Save', exact: true }).click(); }
  await page.getByPlaceholder(/What did you change/).fill('Move weapon scripts');
  await page.getByRole('button', { name: /^Commit \d+ file/ }).click();
  await page.locator('.card').filter({ hasText: 'Move weapon scripts' }).first().waitFor();
  await page.getByRole('button', { name: /Push/ }).click();
  await page.getByText('Up to date with GitHub.').waitFor();
  const remoteLog = execFileSync('git', ['-C', bare, 'log', '--oneline', '-1']).toString();
  if (!remoteLog.includes('Move weapon scripts')) throw new Error('push did not reach remote');
});

await step('Overview summary and edit dialog', async () => {
  await page.getByRole('tab', { name: /Overview/ }).click();
  await page.locator('.ov2-rail').getByText('Activity').first().waitFor();
  await page.locator('.ov2-file').first().waitFor(); // recently changed files
  await page.locator('.ov2-hero').getByRole('button', { name: 'Edit project' }).click();
  await page.locator('.modal textarea').first().fill('IW4X trickshot server mod');
  await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
  await page.locator('.ov-desc').filter({ hasText: 'IW4X trickshot server mod' }).waitFor();
});

await step('Universal search', async () => {
  // the full results page is reached from Ctrl+K (no separate sidebar entry)
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('instashoot');
  await page.locator('.palette .item').filter({ hasText: 'See all results for' }).click();
  await page.getByPlaceholder(/Search projects, file names/).waitFor();
  await page.locator('.card').filter({ hasText: 'tuner.gsc' }).first().waitFor();
  await page.locator('.card').filter({ hasText: 'Instashoot desync' }).first().waitFor();
});

await step('Command palette', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('server set');
  await page.locator('.palette .item').filter({ hasText: 'Server Setup' }).first().waitFor();
  await page.keyboard.press('Escape');
});

await step('Global search: real paths, duplicates, highlights', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('createserver');
  const items = page.locator('.palette .item.fileitem').filter({ hasText: 'createserver.menu' });
  await items.nth(1).waitFor();
  if ((await items.count()) !== 2) throw new Error('expected both identically named files');
  const subs = (await items.locator('.ppath').allTextContents()).map((t) => t.replace(/\s+/g, ' ').trim());
  if (!subs.some((t) => t.includes('OMEN v2 / scripts / mp / createserver.menu')) || !subs.some((t) => t.includes('OMEN v2 / menus / createserver.menu'))) throw new Error('paths: ' + JSON.stringify(subs));
  const titles = await items.evaluateAll((els) => els.map((e) => e.getAttribute('title') || e.getAttribute('data-tip')));
  for (const t of titles) if (!existsSync(t)) throw new Error('absolute path does not exist: ' + t);
  await items.first().locator('mark').first().waitFor();
});

await step('Global search: right-click menu and copy paths', async () => {
  const item = page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' });
  await item.click({ button: 'right' });
  await page.locator('.ctx').waitFor();
  if (!(await page.locator('.palette').isVisible())) throw new Error('right-click must not open/close anything');
  for (const l of ['Reveal in project', 'Show in Windows Explorer', 'Copy full path', 'Copy relative path', 'Rename…', 'Remove from Mothball…', 'Delete file from disk…']) await page.locator('.ctx button').filter({ hasText: l }).waitFor();
  await item.hover();
  if (shots) await page.screenshot({ path: join(shots, 'x-search-menu.png') });
  await page.locator('.ctx button').filter({ hasText: 'Copy relative path' }).click();
  if ((await page.evaluate(() => navigator.clipboard.readText())) !== 'menus/createserver.menu') throw new Error('relative path');
  await item.click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Copy full path' }).click();
  const full = await page.evaluate(() => navigator.clipboard.readText());
  if (full !== join(omen, 'menus/createserver.menu')) throw new Error('full path: ' + full);
  await item.click({ button: 'right' });
  await page.locator('.ctx').waitFor();
  await page.keyboard.press('Escape'); // closes the menu only
  await page.locator('.ctx').waitFor({ state: 'detached' });
  if (!(await page.locator('.palette').isVisible())) throw new Error('Escape on the menu closed the palette');
  await page.keyboard.press('Escape');
  await page.locator('.palette').waitFor({ state: 'detached' });
});

await step('3D printing project', async () => {
  const pr = (await api('projects.create', { name: 'Desk Cat', category: '3d-printing', linkPath: deskCat })).data;
  if (!pr?.id) throw new Error('create failed');
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').filter({ hasText: 'Desk Cat' }).click();
});
await step('3D printing project details', async () => {
    await page.getByText('Print details').waitFor();
    await page.locator('[data-panel=details]').getByRole('button', { name: 'Edit' }).click();
    await page.getByPlaceholder('Notes (settings, what went wrong…)').fill('Gray layer showed through');
    await page.getByRole('button', { name: 'Log', exact: true }).click();
    await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
    await page.getByText('Gray layer showed through').waitFor();
    if (shots) await page.screenshot({ path: join(shots, 'x-3d-overview.png') });
    await page.locator('.ov2-groups .chip').filter({ hasText: 'Meshes' }).click();
    await page.locator('.chip.on').filter({ hasText: 'Meshes' }).waitFor();
    await page.locator('.filerow').filter({ hasText: 'desk cat.stl' }).click();
    await page.getByText('Mesh bounds').waitFor();
    if (await page.locator('.filerow').filter({ hasText: 'README.md' }).count()) throw new Error('type filter not applied');
    // 3D preview (large files load on request)
    const show = page.getByRole('button', { name: 'Show 3D preview' });
    await show.or(page.locator('.mv-bar')).first().waitFor({ timeout: 30000 });
    if (await show.count()) await show.click();
    await page.locator('.mv-bar').getByText(/triangles/).waitFor({ timeout: 60000 });
    if (shots) await page.locator('.modelview').screenshot({ path: join(shots, 'x-3d-preview.png') });
    await page.locator('.chip').filter({ hasText: 'All files' }).click();
    await page.getByRole('button', { name: /^Backups & versions/ }).click();
    await page.locator('.verrow').first().waitFor();
    if (shots) await page.screenshot({ path: join(shots, 'x-3d-files.png') });
  });

await step('Reveal in project from another project', async () => {
  await page.locator('.ptab.on').filter({ hasText: 'Desk Cat' }).waitFor();
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('createserver');
  const item = page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' });
  await item.click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Reveal in project' }).click();
  await page.locator('.ptab.on').filter({ hasText: 'OMEN v2' }).waitFor();
  await page.locator('.tabs button.on').filter({ hasText: 'Files' }).waitFor();
  await page.locator('.tnode.on').filter({ hasText: 'menus' }).waitFor();
  await page.locator('.filerow.sel').filter({ hasText: 'createserver.menu' }).waitFor();
  await page.locator('.doc .mono').filter({ hasText: 'menus/createserver.menu' }).first().waitFor();
});

await step('Global search: keyboard open', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('createserver');
  await page.locator('.palette .item.fileitem').nth(1).waitFor();
  const idx = await page.locator('.palette .item').evaluateAll((els) => els.findIndex((e) => e.textContent.includes('scripts / mp')));
  for (let i = 0; i < idx; i++) await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await page.locator('.filerow.sel').filter({ hasText: 'createserver.menu' }).waitFor();
  await page.locator('.doc .mono').filter({ hasText: 'scripts/mp/createserver.menu' }).first().waitFor();
});

await step('Global search: rename', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('createserver');
  await page.locator('.palette .item.fileitem').filter({ hasText: 'scripts / mp / createserver.menu' }).click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Rename…' }).click();
  const inp = page.locator('.modal .mbody input');
  await inp.fill('create_server.menu');
  await inp.press('Enter');
  await page.locator('.palette input').fill('create_server');
  await page.locator('.palette .item.fileitem').filter({ hasText: 'scripts / mp / create_server.menu' }).waitFor();
  // substring search: "server" finds both create_server.menu and createserver.menu
  await page.locator('.palette input').fill('server');
  await page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' }).waitFor();
  if (!existsSync(join(omen, 'scripts/mp/create_server.menu')) || existsSync(join(omen, 'scripts/mp/createserver.menu'))) throw new Error('rename not on disk');
});

await step('Global search: safe delete', async () => {
  await page.locator('.palette input').fill('createserver');
  await page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' }).click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Delete file from disk…' }).click();
  await page.locator('.modal').getByText('Recycle Bin', { exact: false }).or(page.locator('.modal').getByText('trash', { exact: false })).first().waitFor();
  await page.locator('.modal').getByRole('button', { name: 'Delete' }).click();
  await page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' }).waitFor({ state: 'detached' });
  if (existsSync(join(omen, 'menus/createserver.menu'))) throw new Error('file still on disk');
  await page.keyboard.press('Escape');
  // gone from the Files list; restorable from Filter → Deleted
  await page.locator('.tnode').filter({ hasText: '(all files)' }).click();
  if (await page.locator('.filerow').filter({ hasText: 'menus/createserver.menu' }).count()) throw new Error('deleted file still listed');
  await page.getByRole('button', { name: /^Filter/ }).click();
  await page.locator('.dd-item').filter({ hasText: 'Deleted (restorable)' }).click();
  await page.keyboard.press('Escape');
  await page.locator('.filerow').filter({ hasText: 'createserver.menu' }).click();
  await page.locator('.alert').getByRole('button', { name: 'Restore' }).click();
  await page.locator('.doc').getByText('You deleted this file').waitFor({ state: 'detached' });
  if (!existsSync(join(omen, 'menus/createserver.menu'))) throw new Error('restore failed');
});

await step('Files tab context menu still works', async () => {
  await page.getByRole('button', { name: /^Filter/ }).click();
  await page.locator('.dd-item').filter({ hasText: 'All files' }).click();
  await page.keyboard.press('Escape');
  await page.locator('.filerow').filter({ hasText: 'weapons.json' }).click({ button: 'right' });
  for (const l of ['Edit in Mothball', 'Copy relative path', 'Rename…', 'Delete file from disk…', 'Mark reviewed']) await page.locator('.ctx button').filter({ hasText: l }).waitFor();
  await page.keyboard.press('Escape');
});

await step('Global search: clickable folder breadcrumbs', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('createserver');
  const item = page.locator('.palette .item.fileitem').filter({ hasText: 'menus / createserver.menu' });
  await item.locator('.crumb').filter({ hasText: 'menus' }).click();
  await page.locator('.palette').waitFor({ state: 'detached' });
  await page.locator('.tnode.on').filter({ hasText: 'menus' }).waitFor();
  await page.locator('.filerow').filter({ hasText: 'createserver.menu' }).waitFor();
});

await step('Settings: save behaviour and backup storage location', async () => {
  await page.getByRole('button', { name: /Settings/ }).click();
  const section = (name) => page.locator('.set-link').filter({ hasText: name }).click();
  await section('Editor & saving');
  await page.locator('.set-title').filter({ hasText: 'Editor & saving' }).waitFor();
  await page.locator('label').filter({ hasText: 'Just save' }).locator('input').check();
  if (await page.evaluate(() => localStorage.getItem('pv.saveMode')) !== 'manual') throw new Error('save mode not stored');
  await page.locator('label').filter({ hasText: 'Ask every time' }).locator('input').check();
  await section('Storage');
  const card = page.locator('.card').filter({ hasText: 'Backup storage location' });
  await card.getByText('available').waitFor();
  const newRoot = join(tmp, 'ExternalDrive', 'PVBackups');
  page.once('dialog', (d) => d.accept(newRoot));
  await card.getByRole('button', { name: 'Change…' }).click();
  await page.locator('.modal').getByRole('button', { name: 'Copy and switch' }).click();
  await card.getByText(newRoot).waitFor();
  if (!existsSync(join(newRoot, 'objects')) || !existsSync(join(newRoot, 'history'))) throw new Error('backups not copied to the new location');
  const sidecars = readdirSync(join(newRoot, 'history'), { recursive: true }).filter((f) => String(f).endsWith('.json'));
  if (!sidecars.length) throw new Error('no sidecar metadata');
  await toastGone();
});

await step('Settings: move managed project folders to another drive', async () => {
  // a managed project ("Kept in app storage") with a file in it
  const pr = (await api('projects.create', { name: 'Kept In App' })).data;
  const oldRoot = pr.rootPath;
  writeFileSync(join(oldRoot, 'part.stl'), 'solid x\nendsolid x\n');
  await api('files.scan', { projectId: pr.id });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.set-link').filter({ hasText: 'Storage' }).click();
  const card = page.locator('.card').filter({ hasText: 'Project folders' });
  await card.getByText('default').waitFor();
  const target = join(tmp, 'DDrive', 'Mothball Projects');
  page.once('dialog', (d) => d.accept(target));
  await card.getByRole('button', { name: 'Change…' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Move project folders?' });
  if (!(await m.getByRole('checkbox').isChecked())) throw new Error('moving existing projects should be the default');
  if (shots) await page.screenshot({ path: join(shots, 'x-move-projects.png') });
  await m.getByRole('button', { name: 'Move projects' }).click();
  await card.getByText(/Moved 1 project and freed \d+ B in the old location/).waitFor({ timeout: 30000 });
  await card.getByText(target).waitFor();
  const moved = (await api('projects.get', { id: pr.id })).data;
  if (!moved.rootPath.startsWith(target) || existsSync(oldRoot)) throw new Error(`not moved: ${moved.rootPath}, old exists: ${existsSync(oldRoot)}`);
  if (readFileSync(join(moved.rootPath, 'part.stl'), 'utf8') !== 'solid x\nendsolid x\n') throw new Error('file content');
  // new managed projects go there; removing with "delete folder" works there too
  const p2 = (await api('projects.create', { name: 'Also There' })).data;
  if (!p2.rootPath.startsWith(target)) throw new Error('new project not created in the new location: ' + p2.rootPath);
  for (const id of [pr.id, p2.id]) if (!(await api('projects.remove', { id, deleteManagedFiles: true })).ok) throw new Error('remove failed');
  await page.getByRole('button', { name: 'Dashboard' }).click();
});

await step('New project as a copy; attach a dependency, put it in place, put the original back', async () => {
  // an existing folder, added as a copy: the original must stay untouched
  const orig = join(tmp, 'OMEN old');
  mkdirSync(join(orig, 'scripts'), { recursive: true });
  writeFileSync(join(orig, 'scripts', 'menu.gsc'), 'init() {}\n');
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.getByRole('button', { name: 'New project' }).last().click();
  const dlg = page.locator('.modal').filter({ hasText: 'New project' });
  page.once('dialog', (d) => d.accept(orig));
  await dlg.getByRole('button', { name: /Browse/ }).click();
  await dlg.locator('label').filter({ hasText: 'Make a copy in Mothball' }).click();
  await dlg.getByRole('button', { name: 'Create project' }).click();
  await page.locator('.ptab.on').filter({ hasText: 'OMEN old' }).waitFor({ timeout: 30000 });
  const pr = (await api('projects.list')).data.find((x) => x.name === 'OMEN old');
  if (pr.mode !== 'managed' || pr.rootPath.startsWith(orig)) throw new Error('not copied: ' + JSON.stringify(pr));
  if (readFileSync(join(pr.rootPath, 'scripts', 'menu.gsc'), 'utf8') !== 'init() {}\n') throw new Error('copy content');
  writeFileSync(join(pr.rootPath, 'scripts', 'menu.gsc'), 'edited copy\n');
  if (readFileSync(join(orig, 'scripts', 'menu.gsc'), 'utf8') !== 'init() {}\n') throw new Error('original changed');
  await api('projects.update', { id: pr.id, patch: { category: 'addon', module: { subtype: 'iw4x' } } });

  // a game folder with the stock DLL, and the modded one this version needs
  const game = join(tmp, 'MW2'); mkdirSync(game, { recursive: true });
  writeFileSync(join(game, 'iw4x.dll'), 'stock');
  const modded = join(tmp, 'downloads', 'iw4x-old.dll'); mkdirSync(join(tmp, 'downloads'), { recursive: true });
  writeFileSync(modded, 'old modded build');
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').filter({ hasText: 'OMEN old' }).click();
  await page.getByRole('tab', { name: /Overview/ }).click();
  const panel = page.locator('[data-panel=deps]');
  await panel.getByText(/Attach files this project needs/).waitFor();
  await panel.getByRole('button', { name: 'Attach dependency…' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Attach a dependency' });
  page.once('dialog', (d) => d.accept(modded));
  await m.getByRole('button', { name: 'Browse…' }).click();
  await m.getByPlaceholder(/leave empty to just keep it/).fill(join(game, 'iw4x.dll'));
  await m.getByPlaceholder('iw4x.dll (modded)').fill('iw4x.dll (old build)');
  await m.getByRole('button', { name: 'Attach' }).click();
  const row = panel.locator('.dep-row').filter({ hasText: 'iw4x.dll (old build)' });
  await row.getByText('A different file is in place').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-deps.png') });
  await row.getByRole('button', { name: 'Put in place' }).click();
  await row.getByText('In place', { exact: true }).waitFor();
  if (readFileSync(join(game, 'iw4x.dll'), 'utf8') !== 'old modded build') throw new Error('dll not installed');
  await toastGone();
  await row.getByRole('button', { name: /Put the original file back/ }).click();
  await page.locator('.modal').getByRole('button', { name: 'Put it back' }).click();
  await row.getByText('A different file is in place').waitFor();
  if (readFileSync(join(game, 'iw4x.dll'), 'utf8') !== 'stock') throw new Error('original not restored');
  await toastGone();
  // an IW4X mod gets IW4X details, not WoW ones; switching the subtype switches the fields
  await page.locator('[data-panel=details]').getByText('IW4X mod details').waitFor();
  await page.locator('[data-panel=details]').getByRole('button', { name: 'Edit' }).click();
  const ed = page.locator('.modal');
  await ed.getByText('IW4X client build').waitFor();
  await ed.getByText('Who needs it').waitFor();
  if (await ed.getByText('Saved variables').count()) throw new Error('WoW field shown for an IW4X mod');
  await ed.getByRole('button', { name: 'General' }).click();
  // the type picker: a menu; Game addon / mod opens a submenu with its subtypes
  await ed.getByRole('button', { name: 'Project type: IW4X / MW2 mod' }).click();
  const tmenu = page.getByRole('menu', { name: 'Project type' });
  await tmenu.getByRole('menuitem', { name: 'Game addon / mod' }).hover();
  const flyout = page.getByRole('menu', { name: 'Game addon / mod' });
  const subs = await flyout.getByRole('menuitemradio').allTextContents();
  if (subs.join(', ') !== 'World of Warcraft addon, IW4X / MW2 mod, Other game mod') throw new Error('subtypes: ' + JSON.stringify(subs));
  if ((await flyout.getByRole('menuitemradio', { name: 'IW4X / MW2 mod' }).getAttribute('aria-checked')) !== 'true') throw new Error('current subtype not marked');
  if (shots) await page.screenshot({ path: join(shots, 'x-type-menu.png') });
  // Escape closes only the menu, not the dialog
  await page.keyboard.press('Escape');
  await tmenu.waitFor({ state: 'detached' });
  await ed.getByRole('button', { name: 'Project type: IW4X / MW2 mod' }).waitFor();
  // keyboard: open, down to the addon row, right into the submenu, pick WoW
  await ed.getByRole('button', { name: 'Project type: IW4X / MW2 mod' }).focus();
  await page.keyboard.press('ArrowDown');
  await tmenu.waitFor();
  await page.waitForFunction(() => document.activeElement?.getAttribute('data-parent') === 'addon');
  await page.keyboard.press('ArrowRight');
  await flyout.waitFor();
  await page.keyboard.press('Enter');
  await ed.getByRole('button', { name: /WoW addon details/ }).click();
  await ed.getByText('Saved variables').waitFor();
  if (await ed.getByText('IW4X client build').count()) throw new Error('IW4X field shown for a WoW addon');
  await ed.getByRole('button', { name: 'Cancel' }).click();

  // your own type, with your own fields
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.set-link').filter({ hasText: 'Project types' }).click();
  await page.locator('.typeopt.sub').filter({ hasText: 'IW4X / MW2 mod' }).waitFor();
  await page.getByRole('button', { name: 'New type' }).click();
  await page.getByPlaceholder('Plutonium T6 mod').fill('Plutonium T6 mod');
  await page.locator('.printer-form select').selectOption('addon');
  await page.getByPlaceholder('e.g. Server name').fill('Server name');
  await page.getByRole('button', { name: 'Add field' }).click();
  await page.getByRole('textbox', { name: 'Field 2' }).fill('Patch notes');
  await page.locator('.printer-form label').filter({ hasText: 'Long text' }).nth(1).locator('input').check();
  await page.getByRole('button', { name: 'Create type' }).click();
  await page.locator('.printer-set-row').filter({ hasText: 'Plutonium T6 mod' }).getByText(/based on Game addon \/ mod · 2 fields/).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-custom-types.png') });
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').filter({ hasText: 'OMEN old' }).click();
  await page.locator('[data-panel=details]').getByRole('button', { name: 'Edit' }).click();
  await ed.getByRole('button', { name: 'General' }).click();
  await ed.getByRole('button', { name: /^Project type:/ }).click();
  await page.getByRole('menu', { name: 'Project type' }).getByRole('menuitem', { name: 'Game addon / mod' }).click();
  await page.getByRole('menu', { name: 'Game addon / mod' }).getByRole('menuitemradio', { name: 'Plutonium T6 mod' }).click();
  await ed.getByRole('button', { name: /Plutonium T6 mod details/ }).click();
  await ed.getByText('Server name').waitFor();
  await ed.locator('.field').filter({ hasText: 'Server name' }).locator('input').fill('Trickshot Lobby #1');
  await ed.getByRole('button', { name: 'Save' }).click();
  await page.locator('[data-panel=details]').getByText('Trickshot Lobby #1').waitFor();
  await page.locator('[data-panel=about]').getByText('Plutonium T6 mod').waitFor();
  if (!(await api('projects.remove', { id: pr.id, deleteManagedFiles: true })).ok) throw new Error('remove failed');
  await page.getByRole('button', { name: 'Dashboard' }).click();
});

await step('Backup with project files as .zip, verify', async () => {
  await page.getByRole('button', { name: /Settings/ }).click();
  await page.locator('.set-link').filter({ hasText: 'Backups' }).click();
  const card = page.locator('.card').filter({ hasText: 'Create backup now' });
  await card.getByPlaceholder('E:\\Backups\\Mothball', { exact: true }).fill(backups);
  await card.locator('select').nth(1).selectOption('zip');
  await card.locator('label').filter({ hasText: 'Include the project files themselves' }).locator('input[type=checkbox]').check();
  await card.getByRole('button', { name: 'Create backup now' }).click();
  await card.getByText(/files verified, including \d+ project files/).waitFor({ timeout: 60000 });
  const zips = readdirSync(backups).filter((f) => f.endsWith('.zip'));
  if (zips.length !== 1) throw new Error('expected one .zip backup: ' + readdirSync(backups));
  await toastGone();
});

await step('Restore: check a backup, restore a deleted project file', async () => {
  const zip = join(backups, readdirSync(backups).find((f) => f.endsWith('.zip')));
  const target = join(omen, 'configs/weapons.json');
  const before = readFileSync(target, 'utf8');
  const { rmSync } = await import('node:fs');
  rmSync(target);
  await page.locator('.set-link').filter({ hasText: 'Restore' }).click();
  const card = page.locator('.card').filter({ hasText: 'Check or restore a backup' });
  await card.getByPlaceholder(/Mothball-backup-/).fill(zip);
  await card.getByRole('button', { name: 'Check', exact: true }).click();
  await card.getByText(/Verified — \d+ files match/).waitFor({ timeout: 60000 });
  await card.locator('tr').filter({ hasText: 'OMEN v2' }).getByRole('button', { name: /Restore files/ }).click();
  const m = page.locator('.modal').filter({ hasText: 'Restore files of' });
  await m.getByText(/missing file\(s\) will be recreated/).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-restore-files.png') });
  await m.getByRole('button', { name: /Restore \d+ file/ }).click();
  await page.locator('.modal').filter({ hasText: 'Files restored' }).getByRole('button', { name: 'Done' }).click();
  if (readFileSync(target, 'utf8') !== before) throw new Error('file not restored');
  // the database restore is staged, then can be cancelled
  await card.getByRole('button', { name: /Restore the vault database/ }).click();
  await page.locator('.modal').getByRole('button', { name: 'Restore on next start' }).click();
  await card.getByText(/will be applied the next time/).waitFor();
  await card.getByRole('button', { name: 'Cancel it' }).click();
  await card.getByText(/will be applied the next time/).waitFor({ state: 'detached' });
  await toastGone();
});

await step('Scheduled backups', async () => {
  await page.locator('.set-link').filter({ hasText: 'Backups' }).click();
  const card = page.locator('.card').filter({ hasText: 'Scheduled backups' });
  const sched = join(tmp, 'scheduled'); mkdirSync(sched);
  await card.getByPlaceholder('E:\\Backups\\Mothball', { exact: true }).fill(sched);
  await card.getByRole('button', { name: 'Save schedule' }).click();
  await card.locator('label').filter({ hasText: 'Back up automatically' }).locator('input').check();
  await page.getByText('Backup schedule saved').first().waitFor();
  await card.getByRole('button', { name: 'Run now' }).click();
  await page.getByText(/Scheduled backup written/).waitFor({ timeout: 60000 });
  if (!readdirSync(sched).some((f) => f.includes('-auto'))) throw new Error('no scheduled backup written');
  await card.getByText(/Last run .* OK/).waitFor();
  await toastGone();
});

await step('GSC: functions list, go to definition, function search', async () => {
  writeFileSync(join(omen, 'scripts/mp/_hud.gsc'), '#include scripts\\mp\\_util;\n\ninit()\n{\n    level thread scripts\\mp\\_util::util();\n}\n');
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').filter({ hasText: 'OMEN v2' }).click();
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.getByRole('button', { name: 'Rescan' }).click();
  await page.locator('.filerow').filter({ hasText: '_hud.gsc' }).click();
  const outline = page.locator('.docsec').filter({ hasText: 'Functions' });
  await outline.locator('.symname').filter({ hasText: 'init()' }).waitFor();
  await outline.locator('.chip').filter({ hasText: 'scripts\\mp\\_util' }).waitFor();
  if (shots) await outline.screenshot({ path: join(shots, 'x-gsc-outline.png') });
  // editor: F12 on the call jumps to _util.gsc
  await page.getByRole('button', { name: 'Edit file' }).click();
  const ed = page.locator('.pv-editor textarea').first();
  await ed.waitFor();
  // Monaco colours `scripts\mp\_util::util` as one piece; click on its last word, as a person would
  const monacoCall = page.locator('.pv-editor .monaco-editor .view-line span span:visible').filter({ hasText: /::util$/ });
  if (await page.locator('.pv-editor .monaco-editor:visible').count()) {
    await monacoCall.first().waitFor();
    const b = await monacoCall.first().boundingBox();
    await page.mouse.click(b.x + b.width - 6, b.y + b.height / 2);
  } else {
    await ed.evaluate((ta) => { const i = ta.value.indexOf('::util') + 4; ta.focus(); ta.setSelectionRange(i, i); });
  }
  await page.keyboard.press('F12');
  await page.locator('.editor-pane .panehead b').filter({ hasText: '_util.gsc' }).waitFor();
  // uses of util() are listed from the defining file
  // go-to-definition opened _util.gsc in a second tab
  await page.locator('.edtab').filter({ hasText: '_hud.gsc' }).waitFor();
  await page.locator('.edslot:visible').locator('[title="Back to file notes"], [data-tip="Back to file notes"]').click();
  const o2 = page.locator('.docsec').filter({ hasText: 'Functions' });
  await o2.locator('.symrow').filter({ hasText: 'util()' }).getByRole('button', { name: 'Uses' }).click();
  await o2.locator('.refrow').filter({ hasText: '_hud.gsc:5' }).waitFor();
  // Ctrl+K "@util" finds the function and opens it
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('@util');
  await page.locator('.palette .item').filter({ hasText: 'util()' }).first().click();
  await page.locator('.editor-pane .panehead b').filter({ hasText: '_util.gsc' }).waitFor();
  // close every editor tab (nothing unsaved)
  while (await page.locator('.edtab .edtab-x').count()) await page.locator('.edtab .edtab-x').first().click();
});

await step('Files: group the list by folder', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').filter({ hasText: 'OMEN v2' }).click();
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.getByRole('button', { name: /^Sort:/ }).click();
  await page.locator('.dd-item').filter({ hasText: 'Folder' }).click();
  await page.keyboard.press('Escape');
  const sec = page.locator('.filesec').filter({ hasText: 'scripts/mp/' });
  await sec.waitFor();
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-files-grouped.png') });
  await sec.click(); // collapse
  await page.locator('.filerow').filter({ hasText: '_util.gsc' }).waitFor({ state: 'detached' });
  await page.getByRole('button', { name: /^Sort:/ }).click();
  await page.locator('.dd-item').filter({ hasText: 'No grouping' }).click();
  await page.keyboard.press('Escape');
  await page.locator('.filesec').first().waitFor({ state: 'detached' });
});

await step('Tooltips', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('[title="List"], [data-tip="List"]').first().hover();
  await page.locator('.pv-tip').filter({ hasText: 'List' }).waitFor();
  await page.mouse.move(5, 500);
  await page.locator('.pv-tip').waitFor({ state: 'detached' });
});

await step('Dashboard: change a project picture', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  const card = page.locator('.pcard').filter({ hasText: 'Desk Cat' });
  await card.locator('.pc-face .pc-img').waitFor();
  await card.hover();
  await card.getByRole('button', { name: 'Picture' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Picture for' });
  const cand = m.locator('.pic-cand').filter({ hasText: 'Green and yellow cat.png' });
  await cand.locator('img').waitFor({ timeout: 30000 });
  await cand.click();
  await m.getByRole('button', { name: 'Show the whole picture' }).click();
  if (shots) await page.screenshot({ path: join(shots, 'x-picture-dialog.png') });
  await m.getByRole('button', { name: 'Save' }).click();
  await m.waitFor({ state: 'detached' });
  await page.waitForFunction(() => { const i = [...document.querySelectorAll('.pcard')].find((c) => c.textContent.includes('Desk Cat'))?.querySelector('.pc-face .pc-img'); return i && i.style.objectFit === 'contain'; });
  // zoom in on part of the picture; the card uses the same zoom
  await card.hover();
  await card.getByRole('button', { name: 'Picture' }).click();
  await m.getByRole('button', { name: 'Fill the card' }).click();
  await m.getByRole('slider', { name: 'Zoom' }).fill('2');
  await m.getByText('200%').waitFor();
  await m.getByRole('button', { name: 'Save' }).click();
  await m.waitFor({ state: 'detached' });
  await page.waitForFunction(() => { const i = [...document.querySelectorAll('.pcard')].find((c) => c.textContent.includes('Desk Cat'))?.querySelector('.pc-face .pc-img'); return i && i.style.transform === 'scale(2)' && i.style.objectFit === 'cover'; });
  // and back to no picture: the card lists recent files
  await card.hover();
  await card.getByRole('button', { name: 'Picture' }).click();
  await m.locator('.opcard').filter({ hasText: 'No picture' }).click();
  await m.getByRole('button', { name: 'Save' }).click();
  await card.locator('.pc-recent').waitFor();
  await toastGone();
});

// ---- a pretend Bambu printer (plain MQTT; the desktop app uses TLS, tested in crates/bambu-tls)
const fakePrinter = await new Promise((resolve) => {
  const clients = new Set();
  let last = null;
  const pkt = (kind, body) => { const len = []; let n = body.length; do { let b = n % 128; n = Math.floor(n / 128); if (n) b |= 0x80; len.push(b); } while (n); return Buffer.concat([Buffer.from([kind, ...len]), body]); };
  const str = (x) => { const b = Buffer.from(x); return Buffer.concat([Buffer.from([b.length >> 8, b.length & 255]), b]); };
  const send = (sock, obj) => sock.write(pkt(0x30, Buffer.concat([str('device/FAKESERIAL01/report'), Buffer.from(JSON.stringify({ print: obj }))])));
  const srv = createServer((sock) => {
    let buf = Buffer.alloc(0);
    sock.on('error', () => {});
    sock.on('close', () => clients.delete(sock));
    sock.on('data', (d) => {
      buf = Buffer.concat([buf, d]);
      while (buf.length >= 2) {
        let len = 0, mul = 1, i = 1;
        for (;;) { const b = buf[i++]; len += (b & 127) * mul; if (!(b & 128)) break; mul *= 128; }
        if (buf.length < i + len) break;
        const kind = buf[0] >> 4; buf = buf.subarray(i + len);
        if (kind === 1) sock.write(Buffer.from([0x20, 2, 0, 0]));
        else if (kind === 8) { sock.write(Buffer.from([0x90, 3, 0, 1, 0])); clients.add(sock); if (last) send(sock, last); }
        else if (kind === 12) sock.write(Buffer.from([0xd0, 0]));
      }
    });
  });
  srv.listen(0, '127.0.0.1', () => resolve({
    port: srv.address().port,
    report(obj) { last = { ...(last || {}), ...obj }; for (const c of clients) send(c, obj); },
    close() { for (const c of clients) c.destroy(); srv.close(); },
  }));
});
const started = Math.floor(Date.now() / 1000) - 5400;
const job = (o) => ({ command: 'push_status', gcode_file: '/data/Metadata/plate_1.gcode', nozzle_temper: 220.1, bed_temper: 60, print_error: 0, ...o });
const ams = (r0, r1, now) => ({ ams: { tray_now: now, ams: [{ id: '0', tray: [
  { id: '0', tray_type: 'PLA', tray_sub_brands: 'PLA Basic', tray_color: 'FF6A13FF', remain: r0, tray_weight: '1000' },
  { id: '1', tray_type: 'PETG', tray_sub_brands: 'PETG HF', tray_color: '1F1F1FFF', remain: r1, tray_weight: '1000' },
  { id: '2', tray_type: '', tray_color: '00000000' }] }] }, vt_tray: { id: '254', tray_type: 'TPU', tray_color: '2850E0FF', remain: -1 } });

await step('Printers: add a printer, live status, prints logged to the matching project', async () => {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.set-link').filter({ hasText: 'Printers' }).click();
  await page.getByRole('button', { name: 'Add printer' }).click();
  const f = page.locator('.printer-form');
  await f.getByPlaceholder('X2D').fill('X2D');
  await f.getByPlaceholder('192.168.1.50').fill(`127.0.0.1:${fakePrinter.port}`);
  await f.getByPlaceholder('Printer serial number').fill('fakeserial01');
  await f.getByPlaceholder(/8 characters/).fill('12345678');
  await f.getByRole('button', { name: 'Add printer' }).click();
  const row = page.locator('.printer-set-row').filter({ hasText: 'X2D' });
  await row.getByText('FAKESERIAL01').waitFor(); // serial is normalised
  await row.getByText('Idle').waitFor({ timeout: 20000 }); // connected
  // another Bambu printer announcing itself on the network is offered with one click
  const udp = createSocket('udp4');
  const hello = Buffer.from('NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nLocation: 10.0.0.77\r\nNT: urn:bambulab-com:device:3dprinter:1\r\nUSN: 0938NEWPRINTER\r\nDevModel.bambu.com: N6\r\nDevName.bambu.com: Little Boi\r\n\r\n');
  const announce = () => { for (const port of [2021, 1990]) udp.send(hello, port, '127.0.0.1'); };
  announce();
  const tick = setInterval(announce, 1000);
  const foundRow = page.locator('.found-printers .printer-set-row').filter({ hasText: 'Little Boi' });
  await foundRow.waitFor({ timeout: 15000 });
  clearInterval(tick); udp.close();
  await foundRow.getByRole('button', { name: 'Add' }).click();
  if ((await f.getByPlaceholder('192.168.1.50').inputValue()) !== '10.0.0.77' || (await f.getByPlaceholder('Printer serial number').inputValue()) !== '0938NEWPRINTER') throw new Error('found printer not filled in');
  if (shots) await page.screenshot({ path: join(shots, 'x-printer-found.png') });
  await f.getByRole('button', { name: 'Cancel' }).click();
  if (shots) await page.screenshot({ path: join(shots, 'x-printer-settings.png') });

  // a print starts: the dashboard shows its progress
  // seen from the start (1%), so filament use can be measured from the spool levels
  fakePrinter.report(job({ gcode_state: 'RUNNING', subtask_name: 'Desk Cat', mc_percent: 1, mc_remaining_time: 130, layer_num: 1, total_layer_num: 200, gcode_start_time: String(started), ...ams(80, 50, '0') }));
  await page.waitForTimeout(1500);
  fakePrinter.report({ mc_percent: 45, mc_remaining_time: 72, layer_num: 90 });
  await page.getByRole('button', { name: 'Dashboard' }).click();
  const btn = page.getByRole('button', { name: 'Printers' });
  await btn.getByText(/45%/).waitFor({ timeout: 15000 });
  await btn.click();
  const drawer = page.locator('.drawer');
  await drawer.getByText('Printing Desk Cat · 45% · 1h 12m left').waitFor();
  await drawer.getByText('Layer 90 of 200').waitFor();
  // loaded filament: three spools (empty slot skipped), the feeding one marked
  const live = drawer.locator('.pcard-live .spool');
  await live.filter({ hasText: 'PLA Basic' }).waitFor();
  if ((await live.count()) !== 3) throw new Error('spools: ' + (await live.allTextContents()).join(' | '));
  if (!(await live.filter({ hasText: 'PLA Basic' }).getAttribute('class')).includes('active')) throw new Error('active spool not marked');
  await live.filter({ hasText: 'PLA Basic' }).getByText('80%').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-printer-drawer.png') });

  // it finishes: logged to the Desk Cat project (its model file has the same name)
  fakePrinter.report({ gcode_state: 'FINISH', mc_percent: 100, mc_remaining_time: 0, ...ams(74, 50, '255') });
  const done = drawer.locator('.print-row').filter({ hasText: 'Desk Cat' });
  await done.getByText(/Printed · 1h 3\dm · 60 g/).waitFor({ timeout: 15000 });
  await done.locator('.spool').filter({ hasText: 'PLA Basic' }).getByText('60 g').waitFor();
  await done.getByRole('link', { name: 'Desk Cat' }).waitFor();

  // a print with no matching file fails: unassigned, badge on the button, then assigned
  fakePrinter.report(job({ gcode_state: 'RUNNING', subtask_name: 'Benchy', mc_percent: 3, gcode_start_time: String(started + 6000) }));
  await drawer.getByText(/Printing Benchy/).waitFor({ timeout: 15000 });
  fakePrinter.report({ gcode_state: 'FAILED', print_error: 0x03004000 });
  const bad = drawer.locator('.print-row').filter({ hasText: 'Benchy' });
  await bad.getByText('Failed · error 0300-4000').waitFor({ timeout: 15000 });
  await btn.locator('.dash-badge').filter({ hasText: '1' }).waitFor();
  await bad.getByRole('button', { name: 'Assign…' }).click();
  await bad.getByRole('combobox', { name: 'Project' }).selectOption({ label: 'Desk Cat' });
  await bad.getByRole('button', { name: 'Assign', exact: true }).click();
  await bad.getByRole('link', { name: 'Desk Cat' }).waitFor();
  await btn.locator('.dash-badge').waitFor({ state: 'detached' });
  if (shots) await page.screenshot({ path: join(shots, 'x-printer-history.png') });
  await page.keyboard.press('Escape');

  // both are in the project's print history and timeline
  await page.locator('.pcard').filter({ hasText: 'Desk Cat' }).click();
  await page.getByText('2 succeeded').or(page.getByText('1 succeeded, 1 failed')).first().waitFor();
  await page.getByText('Desk Cat on X2D · PLA Basic').first().waitFor();
  await page.locator('[data-panel=details] .spool').filter({ hasText: 'PLA Basic' }).first().waitFor();
  await page.getByText('Benchy on X2D (error 0300-4000)').first().waitFor();
  await page.getByRole('tab', { name: /Timeline/ }).click();
  await page.getByText('Printed Desk Cat').first().waitFor();
  await page.getByText('Print failed: Benchy').first().waitFor();
  await page.getByRole('tab', { name: /Overview/ }).click();
});

await step('Project tabs: several projects open, each keeps its place', async () => {
  await page.evaluate(() => localStorage.setItem('pv.maxTabs', '10'));
  await page.getByRole('button', { name: 'Dashboard' }).click();
  // close whatever is open, to start clean
  while (await page.locator('.ptab').count()) await page.locator('.ptab .ptab-x').first().click();
  await page.locator('.pcard').filter({ hasText: 'OMEN v2' }).click();
  await page.locator('.ptab.on').filter({ hasText: 'OMEN v2' }).waitFor();
  await page.getByRole('tab', { name: /Files/ }).first().click();
  await page.getByRole('button', { name: 'Dashboard' }).click();
  // a plain click replaces the project in the tab you were in (like a browser)…
  await page.locator('.pcard').filter({ hasText: 'Desk Cat' }).click();
  await page.locator('.ptab.on').filter({ hasText: 'Desk Cat' }).waitFor();
  if ((await page.locator('.ptab').count()) !== 1) throw new Error('a plain click opened another tab: ' + (await page.locator('.ptab').allTextContents()).join(', '));
  await page.locator('.sidebar .navitem').filter({ hasText: 'Prints In Place' }).first().click();
  await page.locator('.ptab.on').filter({ hasText: 'Prints In Place' }).waitFor();
  if ((await page.locator('.ptab').count()) !== 1) throw new Error('the sidebar opened another tab');
  await page.locator('.sidebar .navitem').filter({ hasText: 'Desk Cat' }).first().click();
  // …Ctrl+click opens a new one
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click({ modifiers: ['Control'] });
  await page.locator('.ptab.on').filter({ hasText: 'OMEN v2' }).waitFor();
  await page.getByRole('tab', { name: /Files/ }).first().click();
  await page.locator('.ptab').filter({ hasText: 'Desk Cat' }).click();
  if ((await page.locator('.ptab').allTextContents()).length !== 2) throw new Error('expected two project tabs');
  if (shots) await page.screenshot({ path: join(shots, 'x-project-tabs-two.png') });
  // Ctrl+Tab back to OMEN v2: still on its Files tab
  await page.keyboard.press('Control+Tab');
  await page.locator('.ptab.on').filter({ hasText: 'OMEN v2' }).waitFor();
  await page.locator('.tabs button.on:visible').filter({ hasText: 'Files' }).waitFor();
  await page.locator('.filerow:visible').first().waitFor();
  // the sidebar opens a project in its tab rather than a second copy
  await page.locator('.sidebar .navitem').filter({ hasText: 'Desk Cat' }).first().click();
  if ((await page.locator('.ptab').count()) !== 2) throw new Error('reopening made another tab');
  // tabs come back after a restart
  await page.reload();
  await page.locator('.ptab').filter({ hasText: 'Desk Cat' }).waitFor();
  await page.locator('.ptab').filter({ hasText: 'OMEN v2' }).waitFor();
  // middle-click closes a tab; Ctrl+W closes the one in front
  await page.locator('.ptab').filter({ hasText: 'Desk Cat' }).click({ button: 'middle' });
  await page.locator('.ptab').filter({ hasText: 'Desk Cat' }).waitFor({ state: 'detached' });
  await page.locator('.ptab').filter({ hasText: 'OMEN v2' }).click();
  await page.keyboard.press('Control+w');
  await page.locator('.ptab').first().waitFor({ state: 'detached' });
  await page.locator('.dash-bar').waitFor(); // back on the dashboard
  await page.evaluate(() => localStorage.setItem('pv.maxTabs', '1'));
});

await step('Dashboard: project groups', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').first().waitFor();
  await page.locator('.pcard').filter({ hasText: 'OMEN v2' }).click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Add to a group…' }).click();
  await page.locator('.modal input').fill('MW2 mods');
  await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
  const head = page.locator('.pgroup-head').filter({ hasText: 'MW2 mods' });
  await head.waitFor();
  await page.locator('.pgroup').filter({ has: head }).locator('.pcard').filter({ hasText: 'OMEN v2' }).waitFor();
  await page.locator('.sidebar .sectionbtn').filter({ hasText: 'MW2 mods' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-dashboard-groups.png') });
  // opening a project doesn't reorder the sidebar
  const before = await page.locator('.sidebar .navitem .label').allTextContents();
  await page.locator('.sidebar .navitem').filter({ hasText: 'Prints In Place' }).first().click();
  await page.waitForTimeout(400);
  const after = await page.locator('.sidebar .navitem .label').allTextContents();
  if (before.join('|') !== after.join('|')) throw new Error('sidebar reordered: ' + before + ' -> ' + after);
  // a grouped project still shows under Recent, so it's reachable with its group collapsed
  await page.locator('.sidebar .sectionbtn').filter({ hasText: 'MW2 mods' }).click();
  const labels = await page.locator('.sidebar .scroll .navitem .label').allTextContents();
  if (!labels.includes('OMEN v2')) throw new Error('grouped project missing from Recent: ' + labels.join(', '));
  if (shots) await page.screenshot({ path: join(shots, 'x-sidebar-recent.png') });
  await page.locator('.sidebar .sectionbtn').filter({ hasText: 'MW2 mods' }).click();
});

await step('WoW addon: .toc panel and Lua functions', async () => {
  const wl = join(tmp, 'WhatsLeft');
  mkdirSync(join(wl, 'Modules'), { recursive: true });
  writeFileSync(join(wl, 'WhatsLeft.toc'), "## Interface: 110205\n## Title: |cff33ff99What's Left?|r\n## Version: 2.1.0\n## SavedVariables: WhatsLeftDB\n\nCore.lua\nModules\\Farm.lua\nGone.lua\n");
  writeFileSync(join(wl, 'Core.lua'), 'local WL = {}\nfunction WL:OnEnable()\n  self:Refresh()\nend\nfunction WL:Refresh()\nend\n');
  writeFileSync(join(wl, 'Modules/Farm.lua'), 'WL:OnEnable()\n');
  writeFileSync(join(wl, 'Unused.lua'), '');
  const pr = (await api('projects.create', { name: "What's Left?", category: 'addon', linkPath: wl })).data;
  await api('files.scan', { projectId: pr.id });
  await page.evaluate(() => location.reload());
  await page.locator('.sidebar .navitem').filter({ hasText: "What's Left?" }).first().click();
  await page.getByRole('tab', { name: /Files/ }).click();
  await page.locator('.filerow').filter({ hasText: 'WhatsLeft.toc' }).click();
  const sec = page.locator('.docsec').filter({ hasText: 'Addon' });
  await sec.getByText("What's Left?", { exact: true }).waitFor();
  await sec.getByText('11.2.5 (Retail)').waitFor();
  await sec.locator('.tocrow.missing').filter({ hasText: 'Gone.lua' }).waitFor();
  await sec.getByText(/Not loaded by this .toc/).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-wow-toc.png') });
  await page.locator('.filerow').filter({ hasText: 'Core.lua' }).click();
  await page.locator('.symrow').filter({ hasText: 'function WL:Refresh()' }).waitFor();
  await page.locator('.symrow').filter({ hasText: 'function WL:OnEnable()' }).getByRole('button', { name: 'Uses' }).click();
  await page.locator('.refrow').filter({ hasText: 'Modules/Farm.lua:1' }).waitFor();
});

await step('Deploy a project to a game folder', async () => {
  const dest = join(tmp, 'MW2', 'mods', 'omen');
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click();
  await page.getByRole('button', { name: 'Set up deploy' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Deploy & launch' });
  await m.locator('input.mono').first().fill(dest);
  await m.locator('textarea').fill('configs/');
  await m.getByRole('button', { name: 'Preview' }).click();
  await m.getByText(/new · .*already up to date/).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-deploy-setup.png') });
  await m.getByRole('button', { name: 'Save' }).click();
  await m.waitFor({ state: 'detached' });
  await page.getByRole('button', { name: 'Deploy', exact: true }).click();
  await page.getByText(/Deployed: \d+ copied/).first().waitFor();
  if (!existsSync(join(dest, 'scripts/mp/_util.gsc')) || existsSync(join(dest, 'configs'))) throw new Error('deploy result wrong');
  // a file edited in the game folder is not overwritten without asking
  writeFileSync(join(dest, 'scripts/mp/_util.gsc'), '// edited in game folder');
  appendFileSync(join(omen, 'scripts/mp/_util.gsc'), '\n// newer\n');
  await toastGone();
  await page.getByRole('button', { name: 'Deploy', exact: true }).click();
  const c = page.locator('.modal').filter({ hasText: 'Some files were changed in the destination' });
  await c.getByText('scripts/mp/_util.gsc').waitFor();
  if (readFileSync(join(dest, 'scripts/mp/_util.gsc'), 'utf8') !== '// edited in game folder') throw new Error('overwrote without asking');
  await c.getByRole('button', { name: 'Overwrite and deploy' }).click();
  await page.getByText(/Deployed: 1 copied/).first().waitFor();
  if (!readFileSync(join(dest, 'scripts/mp/_util.gsc'), 'utf8').includes('// newer')) throw new Error('not overwritten');
  await toastGone();
});

await step('Time tracking: estimate, timer, log time', async () => {
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click();
  await page.getByRole('tab', { name: /Overview/ }).click();
  const card = page.locator('[data-panel=time]');
  await card.locator('.tc-big').waitFor();
  // file activity during this test run already counts
  if ((await card.locator('.tc-big').textContent()) === '0 min') throw new Error('no time estimated from file activity');
  await card.getByRole('button', { name: 'Start timer' }).click();
  await page.locator('.timerchip').waitFor();
  await card.locator('.btn.on-rec').click();
  await page.locator('.timerchip').waitFor({ state: 'detached' });
  await card.getByRole('button', { name: 'Log time' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Log time' });
  await m.locator('input[inputmode=decimal]').fill('1.5');
  await m.getByPlaceholder('Modelled the receiver, test print').fill('Playtesting');
  await m.getByRole('button', { name: 'Add' }).click();
  await card.locator('.tc-row').filter({ hasText: 'logged' }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-time.png') });
});

await step('Background watching: outside changes appear without Rescan', async () => {
  await api('watch.setSettings', { patch: { seconds: 5 } });
  await page.locator('.sidebar .navitem').filter({ hasText: "What's Left?" }).first().click();
  await page.getByRole('tab', { name: /Timeline/ }).click();
  writeFileSync(join(tmp, 'WhatsLeft', 'Options.lua'), 'local x = 1\n');
  await page.locator('.tlitem').filter({ hasText: 'File changes detected: 1 added' }).waitFor({ timeout: 60000 });
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.getByRole('button', { name: 'Recently changed' }).click();
  await page.locator('.drawer .ds-row').filter({ hasText: 'Options.lua' }).waitFor({ timeout: 15000 });
  await page.keyboard.press('Escape');
  await page.locator('.drawer').waitFor({ state: 'detached' });
});

await step('Game output: IW4X demos suggestion, missing files grouped by folder, ignore folder', async () => {
  const mod = join(tmp, 'iw4x-mod');
  for (const d of ['demos', 'scripts', 'junk', 'players']) mkdirSync(join(mod, d), { recursive: true });
  for (let i = 1; i <= 5; i++) writeFileSync(join(mod, `demos/auto_${i}.dm_13`), 'demo' + i);
  for (let i = 1; i <= 4; i++) writeFileSync(join(mod, `junk/x${i}.tmp`), 'x' + i);
  writeFileSync(join(mod, 'scripts/a.gsc'), 'init() {}\n');
  writeFileSync(join(mod, 'players/config_mp.cfg'), 'bind x\n');
  const pr = (await api('projects.create', { name: 'Demo Spam', category: 'addon', module: { subtype: 'iw4x' }, linkPath: mod })).data;
  if (!/demos\//.test(pr.ignoreGlobs)) throw new Error('new IW4X project should ignore demos/: ' + pr.ignoreGlobs);
  // like a project made before these defaults
  await api('projects.update', { id: pr.id, patch: { ignoreGlobs: '' } });
  await api('files.scan', { projectId: pr.id });
  for (let i = 1; i <= 4; i++) rmSync(join(mod, `junk/x${i}.tmp`));
  for (let i = 1; i <= 2; i++) rmSync(join(mod, `demos/auto_${i}.dm_13`));
  await api('files.scan', { projectId: pr.id });
  await page.goto(base + '/');
  await page.getByText('Demo Spam', { exact: true }).first().click();
  const att = page.locator('.ov2-sec.attention');
  // the suggestion for game output
  const sug = att.locator('.att-suggest');
  await sug.getByText('Stop tracking game output?').waitFor();
  await sug.locator('.mono').filter({ hasText: 'demos/' }).waitFor();
  if (await sug.getByText('players/').count() !== 1) throw new Error('players/ should only be mentioned as kept');
  // junk/: 4 missing files in one row
  const grp = att.locator('.att-row').filter({ hasText: '4 files missing from disk' });
  await grp.locator('.mono').filter({ hasText: 'junk/' }).waitFor();
  if (await att.getByText('x1.tmp').count()) throw new Error('grouped files should start collapsed');
  await grp.getByRole('button', { name: 'Show files' }).click();
  await att.getByText('x3.tmp').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-attention-grouped.png') });
  // stop tracking the game output
  await sug.getByRole('button', { name: 'Stop tracking' }).click();
  await page.locator('.modal').getByRole('button', { name: 'Stop tracking' }).click();
  await sug.waitFor({ state: 'detached' });
  if (await att.getByText('auto_1.dm_13').count()) throw new Error('missing demos still listed');
  // ignore junk/
  await grp.getByRole('button', { name: 'Ignore folder' }).click();
  await page.locator('.modal').getByRole('button', { name: 'Stop tracking' }).click();
  await grp.waitFor({ state: 'detached' });
  const files = (await api('files.list', { projectId: pr.id, limit: 100 })).data.map((f) => f.path).sort();
  if (JSON.stringify(files) !== JSON.stringify(['players/config_mp.cfg', 'scripts/a.gsc'])) throw new Error('tracked after ignoring: ' + files.join(', '));
  // and the next scan doesn't bring anything back as missing
  writeFileSync(join(mod, 'demos/auto_9.dm_13'), 'new');
  await api('files.scan', { projectId: pr.id });
  if ((await api('files.list', { projectId: pr.id, state: 'missing', limit: 100 })).data.length) throw new Error('missing files after rescan');
  await api('projects.remove', { id: pr.id });
  await page.getByRole('button', { name: 'Dashboard' }).click();
});

await step('Dashboard: remove a managed project and delete its folder', async () => {
  const pr = (await api('projects.create', { name: 'Scratch To Delete', category: 'game' })).data;
  await page.evaluate(() => location.reload());
  await page.getByRole('button', { name: 'Dashboard' }).click();
  const card = page.locator('.pcard').filter({ hasText: 'Scratch To Delete' });
  await card.click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Remove from vault…' }).click();
  const m = page.locator('.modal').filter({ hasText: 'Remove “Scratch To Delete”?' });
  await m.getByRole('button', { name: 'Remove project' }).waitFor();
  await m.locator('.confirm-check input').check();
  if (shots) await page.screenshot({ path: join(shots, 'x-remove-delete.png') });
  await m.getByRole('button', { name: 'Remove and delete files' }).click();
  await m.waitFor({ state: 'detached' });
  await card.waitFor({ state: 'detached' });
  if (!pr.rootPath) throw new Error('no rootPath from projects.create');
  if ((await import('node:fs')).existsSync(pr.rootPath)) throw new Error('managed folder still on disk');
  await toastGone();
});

await step('Dashboard: one top bar, attention and recent in drawers; overview panels rearrange', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  const bar = page.locator('.dash-bar');
  await bar.getByPlaceholder('Search projects…').waitFor();
  await bar.getByRole('button', { name: 'New project' }).waitFor();
  // attention badge -> drawer
  const att = bar.getByRole('button', { name: 'Needs attention' });
  const badge = await att.locator('.dash-badge').textContent().catch(() => '');
  await att.click();
  await page.locator('.drawer').filter({ hasText: 'Needs attention' }).waitFor();
  if (badge && !(await page.locator('.drawer .ds-row').count())) throw new Error('badge but empty drawer');
  await page.waitForTimeout(400);
  if (shots) await page.screenshot({ path: join(shots, 'x-dash-drawer.png') });
  await page.locator('.drawer-scrim').click({ position: { x: 20, y: 300 } });
  await page.locator('.drawer').waitFor({ state: 'detached' });
  // overview panels: drag Activity to the top of the side column, collapse, remembered, reset
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click();
  const rail = () => page.locator('.ov2-rail > .panel').evaluateAll((els) => els.map((e) => e.getAttribute('data-panel')));
  await page.locator('[data-panel=activity] > .panel-h').dragTo(page.locator('[data-panel=time]'), { targetPosition: { x: 40, y: 4 } });
  await page.waitForFunction(() => document.querySelector('.ov2-rail > .panel')?.getAttribute('data-panel') === 'activity');
  await page.locator('[data-panel=activity] .panel-t').click();
  await page.evaluate(() => location.reload());
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click();
  await page.locator('[data-panel=activity].collapsed').waitFor();
  if ((await rail())[0] !== 'activity') throw new Error('layout not remembered: ' + await rail());
  await page.locator('[data-panel=activity] > .panel-h').click({ button: 'right' });
  await page.locator('.ctx button').filter({ hasText: 'Reset layout' }).click();
  await page.waitForFunction(() => document.querySelector('.ov2-rail > .panel')?.getAttribute('data-panel') === 'time');
  await page.getByRole('button', { name: 'Dashboard' }).click();
  // project cards: drag one onto another -> "My order"
  const names = () => page.locator('.pgroup').last().locator('.pcard .name').allTextContents();
  const before = await names();
  const last = page.locator('.pgroup').last().locator('.pcard').last();
  const first = page.locator('.pgroup').last().locator('.pcard').first();
  await last.dragTo(first, { targetPosition: { x: 10, y: 60 } });
  await page.getByText('Sort: My order (drag cards)').waitFor();
  const after = await names();
  if (after[0] !== before[before.length - 1]) throw new Error('card order not changed: ' + after.join(', '));
  // overview panels collapse too
  await page.locator('.sidebar .navitem').filter({ hasText: 'OMEN v2' }).first().click();
  await page.locator('[data-panel=time] .panel-t').click();
  await page.locator('[data-panel=time] .tc-big').waitFor({ state: 'detached' });
  await page.locator('[data-panel=time] .panel-t').click();
  await page.locator('[data-panel=time] .tc-big').waitFor();
  // collapsed panels still say what's in them
  for (const id of ['time', 'next', 'notes', 'activity', 'info']) await page.locator(`[data-panel=${id}] .panel-t`).click();
  await page.locator('[data-panel=activity] .panel-summary').waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'x-collapsed-rail.png') });
  for (const id of ['time', 'next', 'notes', 'activity', 'info']) await page.locator(`[data-panel=${id}] .panel-t`).click();
});

await step('Settings: hide project types from the pickers', async () => {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.set-link').filter({ hasText: 'Project types' }).click();
  await page.getByRole('button', { name: 'Only the types I use' }).click();
  const video = page.locator('.typeopt').filter({ hasText: 'Video editing' });
  if (await video.locator('input').isChecked()) throw new Error('unused type still on');
  if (shots) await page.screenshot({ path: join(shots, 'x-project-types.png') });
  await page.getByRole('button', { name: 'New project' }).first().click();
  await page.locator('.modal').getByRole('button', { name: /^Project type:/ }).click();
  const tm = page.getByRole('menu', { name: 'Project type' });
  const opts = [...(await tm.getByRole('menuitemradio').allTextContents()), ...(await tm.getByRole('menuitem').allTextContents())];
  if (opts.includes('Video editing') || !opts.includes('Software')) throw new Error('picker not filtered: ' + opts.join(', '));
  if (shots) await page.screenshot({ path: join(shots, 'x-type-menu-filtered.png') });
  await page.keyboard.press('Escape');
  await page.locator('.modal').getByRole('button', { name: 'Cancel' }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('button', { name: 'Show all' }).click();
});

await step('Dashboard overview screenshot', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.dash-bar').waitFor();
  await page.evaluate(() => document.querySelector('.content')?.scrollTo(0, 0));
  await page.waitForTimeout(800);
  if (shots) await page.screenshot({ path: join(shots, 'x-dashboard.png') });
});

await browser.close();
srv.kill();
fakePrinter.close();
if (errors.length) { console.error('Browser errors:\n' + errors.join('\n')); process.exit(1); }
console.log('All UI steps passed.');
