// Bumps the version everywhere, commits and tags it.
//   npm run release -- 0.3.0        (then: git push --follow-tags)
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const v = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(v || '')) { console.error('usage: npm run release -- <major.minor.patch>'); process.exit(2); }

const edit = (rel, fn) => { const p = join(root, rel); writeFileSync(p, fn(readFileSync(p, 'utf8'))); };
edit('package.json', (s) => s.replace(/"version": "[^"]+"/, `"version": "${v}"`));
edit('src-tauri/tauri.conf.json', (s) => s.replace(/"version": "[^"]+"/, `"version": "${v}"`));
edit('src-tauri/Cargo.toml', (s) => s.replace(/^version = "[^"]+"/m, `version = "${v}"`));
edit('crates/vault-core/Cargo.toml', (s) => s.replace(/^version = "[^"]+"/m, `version = "${v}"`));

const git = (...a) => execFileSync('git', ['-C', root, ...a], { stdio: 'inherit' });
git('add', 'package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'crates/vault-core/Cargo.toml');
git('commit', '-m', `Release v${v}`);
git('tag', `v${v}`);
console.log(`Tagged v${v}. Push with: git push --follow-tags`);
