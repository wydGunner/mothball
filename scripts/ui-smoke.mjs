// End-to-end UI test: runs the real UI against the real core (via the dev
// server) in headless Chromium and walks through the critical workflows.
//
//   cargo build --manifest-path crates/vault-core/Cargo.toml --bin vault-devserver
//   npm run build:ui
//   node scripts/ui-smoke.mjs [--print-vault <folder with print_vault.sqlite>] [--shots <dir>]
//
// Needs git on PATH. Exits non-zero on the first failed step.
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, appendFileSync, renameSync, existsSync, cpSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const arg = (k) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : undefined; };
const shots = arg('--shots');
const pvSrc = arg('--print-vault');
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
const git = (...a) => execFileSync('git', ['-C', omen, '-c', 'user.name=Dev', '-c', 'user.email=dev@example.com', ...a], { stdio: 'pipe' });
git('init', '-q'); git('add', '-A'); git('commit', '-qm', 'Initial OMEN import');
appendFileSync(join(omen, 'scripts/mp/weapon_tuner.gsc'), '\nonPlayerConnect() {}\n'); git('commit', '-qam', 'Add player connect hook');
let pvDir;
if (pvSrc && existsSync(pvSrc)) { pvDir = join(tmp, 'PV'); cpSync(pvSrc, pvDir, { recursive: true }); }
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

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const errors = [];
page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });
let n = 0;
const step = async (name, fn) => {
  process.stdout.write(`• ${name} … `);
  try { await fn(); console.log('ok'); if (shots) await page.screenshot({ path: join(shots, `${String(++n).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`) }); }
  catch (e) { console.log('FAILED'); console.error(e); console.error(errors.join('\n')); if (shots) await page.screenshot({ path: join(shots, 'FAILED.png') }); await browser.close(); srv.kill(); process.exit(1); }
};
const api = (cmd, args = {}) => page.evaluate(async ([c, a]) => (await (await fetch('/api/' + c, { method: 'POST', body: JSON.stringify(a) })).json()), [cmd, args]);
const toastGone = async () => { while (await page.locator('.toast button').count()) await page.locator('.toast button').first().click(); };

await page.goto(base);

await step('Empty dashboard', async () => { await page.getByText('Your vault is empty').waitFor(); });

await step('Create OMEN v2 linked project', async () => {
  await page.getByRole('button', { name: 'New project' }).first().click();
  await page.getByPlaceholder('D:\\Dev\\OMEN').fill(omen);
  await page.getByPlaceholder('OMEN v2').fill('OMEN v2');
  await page.locator('.modal select').first().selectOption('software');
  await page.getByRole('button', { name: 'Create project' }).click();
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).waitFor();
});

await step('Browse folder tree', async () => {
  await page.locator('.tnode').filter({ hasText: 'configs' }).click();
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).waitFor({ state: 'detached' });
  await page.locator('.filerow').filter({ hasText: 'weapons.json' }).waitFor();
  await page.locator('.tnode').filter({ hasText: '(all files)' }).click();
});

await step('Document weapon_tuner.gsc', async () => {
  await page.locator('.filerow').filter({ hasText: 'weapon_tuner.gsc' }).click();
  await page.getByPlaceholder(/What is this file for/).fill('Handles weapon-specific tuning and behavior');
  await page.locator('textarea.code').fill('- Contains instashoot configuration\n- Depends on [[file:configs/weapons.json]]\n- Requires dedicated server testing');
  await page.getByPlaceholder('e.g. Never edit while the server is running').fill('Requires dedicated server testing before release');
  await page.locator('.savestate').filter({ hasText: 'Saved' }).waitFor({ timeout: 5000 });
  await page.getByRole('button', { name: 'View' }).click();
  await page.locator('.md').getByText('Contains instashoot configuration').waitFor();
});

await step('Link script to related file, note and bug', async () => {
  const pid = (await api('projects.list')).data[0].id;
  const note = await api('notes.create', { projectId: pid, title: 'Weapon Systems', body: 'See [[file:scripts/mp/weapon_tuner.gsc]]' });
  const bug = await api('tasks.create', { projectId: pid, title: 'Instashoot desync on dedicated', kind: 'bug' });
  if (!note.ok || !bug.ok) throw new Error('setup failed');
  for (const [tab, name] of [['File', '_util.gsc'], ['Note', 'Weapon Systems'], ['Task / bug', 'Instashoot desync']]) {
    await page.getByRole('button', { name: 'Add link' }).click();
    await page.locator('.modal .seg button').filter({ hasText: tab }).click();
    await page.locator('.modal .filerow').filter({ hasText: name }).first().click();
    await page.locator('.docsec').getByText(name).first().waitFor();
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
});

await step('Compare file versions (no Git needed)', async () => {
  await page.getByRole('button', { name: /^Versions/ }).click();
  await page.locator('.verrow').nth(1).getByRole('button', { name: /vs now/ }).click();
  await page.locator('.modal .dl.add, .modal .half.add').first().waitFor();
  await page.locator('.modal').getByText('// tweak').first().waitFor();
  await page.locator('.modal').getByRole('button', { name: /Unified/ }).click();
  await page.locator('.modal .dl.add .tx').filter({ hasText: '// tweak' }).waitFor();
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
  await page.locator('.savestate').filter({ hasText: 'Saved' }).waitFor({ timeout: 5000 });
  await page.getByRole('button', { name: 'Note', exact: true }).click();
  await page.locator('input.title[value="Untitled"]').waitFor();
  await page.locator('input.title').fill('Server Setup');
  await page.locator('.savestate').filter({ hasText: 'Saved' }).waitFor({ timeout: 5000 });
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
  await page.getByText('Recent activity').waitFor();
  await page.locator('.card').filter({ hasText: 'No description yet' }).getByRole('button', { name: 'Edit' }).click();
  await page.locator('.modal textarea').first().fill('IW4X trickshot server mod');
  await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
  await page.locator('.ov-desc').filter({ hasText: 'IW4X trickshot server mod' }).waitFor();
});

await step('Universal search', async () => {
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.getByPlaceholder(/Search projects, file names/).fill('instashoot');
  await page.locator('.card').filter({ hasText: 'tuner.gsc' }).first().waitFor();
  await page.locator('.card').filter({ hasText: 'Instashoot desync' }).first().waitFor();
});

await step('Command palette', async () => {
  await page.keyboard.press('Control+K');
  await page.locator('.palette input').fill('server set');
  await page.locator('.palette .item').filter({ hasText: 'Server Setup' }).first().waitFor();
  await page.keyboard.press('Escape');
});

if (pvDir) {
  await step('Import Print Vault', async () => {
    await page.getByRole('button', { name: 'Dashboard' }).click();
    await page.getByRole('button', { name: /Import Print Vault/ }).click();
    await page.getByPlaceholder('D:\\PrintVault\\3D Prints Vault').fill(pvDir);
    await page.locator('.modal').getByRole('button', { name: 'Import' }).click();
    await page.getByText(/project\(s\) imported/).waitFor({ timeout: 30000 });
    await page.locator('.modal').getByRole('button', { name: 'Done' }).click();
  });
  await step('3D printing project details', async () => {
    await page.locator('.pcard').filter({ hasText: 'Cat - Green And Yellow' }).click();
    await page.getByText('Print details').waitFor();
    await page.locator('.fact').filter({ hasText: 'X2D' }).waitFor();
    await page.locator('.card').filter({ hasText: 'Print details' }).getByRole('button', { name: 'Edit' }).click();
    await page.getByPlaceholder('Notes (settings, what went wrong…)').fill('Gray layer showed through');
    await page.getByRole('button', { name: 'Log' }).click();
    await page.locator('.modal').getByRole('button', { name: 'Save' }).click();
    await page.getByText('Gray layer showed through').waitFor();
    if (shots) await page.screenshot({ path: join(shots, 'x-3d-overview.png') });
    await page.locator('.groupcard').filter({ hasText: 'Meshes' }).click();
    await page.locator('.chip.on').filter({ hasText: 'Meshes' }).waitFor();
    await page.locator('.filerow').filter({ hasText: 'fixed cat.stl' }).click();
    await page.getByText('Mesh bounds').waitFor();
    if (await page.locator('.filerow').filter({ hasText: 'README.md' }).count()) throw new Error('type filter not applied');
    await page.getByRole('button', { name: /^Versions/ }).click();
    await page.locator('.verrow').first().waitFor();
    if (shots) await page.screenshot({ path: join(shots, 'x-3d-files.png') });
  });
}

await step('Backup and verify', async () => {
  await page.getByRole('button', { name: /Settings/ }).click();
  await page.getByPlaceholder('E:\\Backups\\ProjectVault', { exact: true }).fill(backups);
  await page.getByRole('button', { name: 'Create backup now' }).click();
  await page.getByText(/files verified/).waitFor({ timeout: 30000 });
  await page.getByRole('button', { name: 'Verify', exact: true }).click();
  await page.getByText(/files match their checksums/).waitFor();
  const dirs = readdirSync(backups);
  if (dirs.length !== 1) throw new Error('expected one backup folder');
});

await step('Dashboard', async () => {
  await page.getByRole('button', { name: 'Dashboard' }).click();
  await page.locator('.pcard').first().waitFor();
});

await browser.close();
srv.kill();
if (errors.length) { console.error('Browser errors:\n' + errors.join('\n')); process.exit(1); }
console.log('All UI steps passed.');
