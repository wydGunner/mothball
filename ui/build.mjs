// Bundles the UI with esbuild into ui/dist (no dev server needed: Tauri loads
// the files directly; `vault-devserver` serves them for browser testing).
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, copyFileSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const watch = process.argv.includes('--watch');

// Prefer project node_modules; fall back to NODE_PATH (CI images with
// preinstalled packages).
const nodePaths = [join(root, 'node_modules'), ...(process.env.NODE_PATH || '').split(':').filter(Boolean)];
const require = createRequire(join(root, 'package.json'));
let esbuild;
for (const p of nodePaths) {
  try { esbuild = createRequire(join(p, 'x.js'))('esbuild'); break; } catch { /* next */ }
}
if (!esbuild) esbuild = require('esbuild');

// If @tauri-apps packages are not installed (offline CI), alias them to small
// shims that call the same Tauri IPC endpoints.
const alias = {};
const hasTauriApi = nodePaths.some((p) => existsSync(join(p, '@tauri-apps', 'api')));
if (!hasTauriApi) {
  console.warn('[build] @tauri-apps packages not installed: using ui/shims (fine for browser tests; run npm install for release builds)');
  alias['@tauri-apps/api/core'] = join(here, 'shims/core.ts');
  alias['@tauri-apps/api/webview'] = join(here, 'shims/webview.ts');
  alias['@tauri-apps/api/window'] = join(here, 'shims/window.ts');
  alias['@tauri-apps/plugin-dialog'] = join(here, 'shims/dialog.ts');
  alias['@tauri-apps/plugin-opener'] = join(here, 'shims/opener.ts');
  alias['@tauri-apps/plugin-updater'] = join(here, 'shims/updater.ts');
  alias['@tauri-apps/plugin-process'] = join(here, 'shims/process.ts');
}

const out = join(here, 'dist');
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
copyFileSync(join(here, 'index.html'), join(out, 'index.html'));

const opts = {
  entryPoints: { app: join(here, 'src/main.tsx'), styles: join(here, 'src/styles.css') },
  outdir: out,
  bundle: true,
  format: 'esm',
  target: 'es2022',
  jsx: 'automatic',
  minify: !watch,
  sourcemap: watch ? 'inline' : false,
  nodePaths,
  alias,
  define: { 'process.env.NODE_ENV': JSON.stringify(watch ? 'development' : 'production') },
  logLevel: 'info',
};

// ---- code editor bundle (Monaco), loaded on demand by the app
const findPkg = (name) => nodePaths.map((p) => join(p, name)).find((p) => existsSync(join(p, 'package.json')));
const monacoDir = findPkg('monaco-editor');
const editorOpts = {
  entryPoints: { monaco: join(here, monacoDir ? 'editor/monaco.ts' : 'editor/fallback.ts') },
  outdir: out, bundle: true, format: 'esm', target: 'es2022', minify: !watch, nodePaths, logLevel: 'warning',
  loader: { '.ttf': 'file' },
};
const workerOpts = monacoDir && {
  entryPoints: {
    'editor.worker': join(monacoDir, 'esm/vs/editor/editor.worker.js'),
    'json.worker': join(monacoDir, 'esm/vs/language/json/json.worker.js'),
    'css.worker': join(monacoDir, 'esm/vs/language/css/css.worker.js'),
    'html.worker': join(monacoDir, 'esm/vs/language/html/html.worker.js'),
    'ts.worker': join(monacoDir, 'esm/vs/language/typescript/ts.worker.js'),
  },
  outdir: out, bundle: true, format: 'iife', target: 'es2022', minify: !watch, logLevel: 'warning',
};
if (!monacoDir) console.warn('[build] monaco-editor not installed: using the plain-text editor stand-in (run npm install for the real editor)');

if (watch) {
  for (const o of [opts, editorOpts, workerOpts].filter(Boolean)) { const ctx = await esbuild.context(o); await ctx.watch(); }
} else {
  await Promise.all([opts, editorOpts, workerOpts].filter(Boolean).map((o) => esbuild.build(o)));
}
