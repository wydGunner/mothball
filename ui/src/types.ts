// Project types: the built-in type (category) decides file groups and which
// files get earlier versions kept; a subtype adds its own detail fields.
// A project's subtype lives in module.subtype: a built-in id ("wow") or
// "custom:<id>" for a type the user made in Settings.
import { useEffect, useState } from 'react';
import { call } from './ipc';
import { CATEGORY_LABELS, hiddenTypes } from './format';

export interface Subtype { id: string; label: string; short: string }
export const SUBTYPES: Record<string, Subtype[]> = {
  addon: [
    { id: 'wow', label: 'World of Warcraft addon', short: 'WoW addon' },
    { id: 'iw4x', label: 'IW4X / MW2 mod', short: 'IW4X mod' },
    { id: 'other', label: 'Other game mod', short: 'Game mod' },
  ],
};

export interface CustomField { key: string; label: string; multiline?: boolean }
export interface CustomType { id: string; name: string; base: string; fields: CustomField[] }

let customTypes: CustomType[] = [];
let loaded = false;
export async function loadCustomTypes(): Promise<CustomType[]> {
  try { customTypes = await call<CustomType[]>('types.custom'); } catch { /* keep what we had */ }
  loaded = true;
  window.dispatchEvent(new CustomEvent('pv:types'));
  return customTypes;
}
export async function saveCustomTypes(types: CustomType[]): Promise<CustomType[]> {
  customTypes = await call<CustomType[]>('types.saveCustom', { types });
  window.dispatchEvent(new CustomEvent('pv:types'));
  return customTypes;
}
export const getCustomTypes = () => customTypes;

/** Re-renders when custom types or hidden types change. */
export function useTypes(): CustomType[] {
  const [, setN] = useState(0);
  useEffect(() => {
    const f = () => setN((n) => n + 1);
    window.addEventListener('pv:types', f);
    if (!loaded) loadCustomTypes();
    return () => window.removeEventListener('pv:types', f);
  }, []);
  return customTypes;
}

/** "addon:wow", "general:custom:ab12cd34" or just "software". */
export function typeKeyOf(category: string, module?: Record<string, any> | null): string {
  let sub: string | undefined = module?.subtype;
  if (!sub && category === 'addon' && module) {
    // projects from before subtypes: WoW if it says so, else leave it general
    if (/warcraft|\bwow\b/i.test(module.game || '') || module.interface || module.savedVariables) sub = 'wow';
  }
  return sub ? `${category}:${sub}` : category;
}
export function splitKey(key: string): { category: string; subtype?: string } {
  const i = key.indexOf(':');
  return i < 0 ? { category: key } : { category: key.slice(0, i), subtype: key.slice(i + 1) };
}
export function customOf(key: string): CustomType | undefined {
  const { subtype } = splitKey(key);
  return subtype?.startsWith('custom:') ? customTypes.find((t) => t.id === subtype.slice(7)) : undefined;
}
export function typeLabel(key: string, short = false): string {
  const { category, subtype } = splitKey(key);
  const c = customOf(key);
  if (c) return c.name;
  const s = subtype && SUBTYPES[category]?.find((x) => x.id === subtype);
  if (s) return short ? s.short : s.label;
  return CATEGORY_LABELS[category] || category;
}

export interface TypeGroup { label: string; options: [string, string][] }
/** Types to offer in pickers, grouped: built-in types with subtypes become a
 *  group of their own; types made in Settings get "Your types". `keep` (the
 *  current type) is always offered so it never disappears. */
export function typeGroups(keep?: string): TypeGroup[] {
  const hidden = new Set(hiddenTypes());
  const ok = (k: string) => !hidden.has(k) || k === keep;
  const plain: [string, string][] = [];
  const groups: TypeGroup[] = [];
  for (const [cat, label] of Object.entries(CATEGORY_LABELS)) {
    const subs = SUBTYPES[cat];
    if (subs) {
      const opts = subs.map((s) => [`${cat}:${s.id}`, s.label] as [string, string]).filter(([k]) => ok(k) && ok(cat));
      if (keep === cat) opts.unshift([cat, `${label} (not specified)`]);
      if (opts.length) groups.push({ label, options: opts });
    } else if (ok(cat)) plain.push([cat, label]);
  }
  const mine = customTypes.map((t) => [`${t.base}:custom:${t.id}`, t.name] as [string, string]).filter(([k]) => ok(k));
  return [{ label: '', options: plain }, ...groups, ...(mine.length ? [{ label: 'Your types', options: mine }] : [])];
}

/** module patch for choosing `key` (keeps everything else in the module). */
export function withType(module: Record<string, any> | undefined, key: string): Record<string, any> {
  const { subtype } = splitKey(key);
  const m = { ...(module || {}) };
  if (subtype) m.subtype = subtype; else delete m.subtype;
  return m;
}
