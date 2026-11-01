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
  alias['@tauri-apps/plugin-dialog'] = join(here, 'shims/dialog.ts');
  alias['@tauri-apps/plugin-opener'] = join(here, 'shims/opener.ts');
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

if (watch) {
  const ctx = await esbuild.context(opts);
  await ctx.watch();
} else {
  await esbuild.build(opts);
}
