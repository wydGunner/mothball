// Black UI with a single-color or analogous palette. Three tones per theme:
//   a1 = primary (buttons, active items, additions)
//   a2 = secondary (secondary highlights, folders, "changed")
//   a3 = tertiary (urgent, removed lines, errors)
// Every theme's tones are shades of one hue or neighbours on the color
// wheel, so nothing clashes.
export interface Theme { id: string; name: string; a1: string; a2: string; a3: string }

export const THEMES: Theme[] = [
  { id: 'mono', name: 'Mono', a1: '#ededed', a2: '#a3a3a3', a3: '#d4d4d4' },
  { id: 'sunset', name: 'Sunset', a1: '#ff7a1a', a2: '#ffb224', a3: '#f0384b' },
  { id: 'ember', name: 'Ember', a1: '#ff6a1a', a2: '#ff9a5c', a3: '#c2410c' },
  { id: 'gold', name: 'Gold', a1: '#ffc53d', a2: '#e5a50a', a3: '#ff8b3d' },
  { id: 'twilight', name: 'Twilight', a1: '#8b5cf6', a2: '#3b82f6', a3: '#ec4899' },
  { id: 'violet', name: 'Violet', a1: '#a78bfa', a2: '#7c3aed', a3: '#c084fc' },
  { id: 'ocean', name: 'Ocean', a1: '#38bdf8', a2: '#6366f1', a3: '#22d3ee' },
  { id: 'blue', name: 'Blue', a1: '#3b82f6', a2: '#60a5fa', a3: '#1d4ed8' },
  { id: 'forest', name: 'Forest', a1: '#22c55e', a2: '#a3e635', a3: '#14b8a6' },
  { id: 'rose', name: 'Rose', a1: '#f43f5e', a2: '#fb7185', a3: '#be123c' },
];

const KEY = 'pv.theme';

function luminance(hex: string) {
  const n = parseInt(hex.slice(1), 16);
  const c = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}

export function getTheme(): Theme {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const t = JSON.parse(raw);
      const ok = (x: unknown) => typeof x === 'string' && /^#[0-9a-f]{6}$/i.test(x);
      if (ok(t.a1) && ok(t.a2) && ok(t.a3)) return { id: String(t.id || 'custom'), name: String(t.name || 'Custom'), a1: t.a1, a2: t.a2, a3: t.a3 };
    }
  } catch { /* storage unavailable or bad value */ }
  return THEMES[0];
}

export function applyTheme(t: Theme) {
  const r = document.documentElement.style;
  r.setProperty('--accent', t.a1);
  r.setProperty('--accent2', t.a2);
  r.setProperty('--accent3', t.a3);
  r.setProperty('--accent-fg', luminance(t.a1) > 0.35 ? '#000000' : '#ffffff');
  try { localStorage.setItem(KEY, JSON.stringify(t)); } catch { /* ignore */ }
  window.dispatchEvent(new CustomEvent('pv-theme', { detail: t }));
}

/** A custom single-hue theme from one color: the color plus a lighter and a darker shade. */
export function themeFromColor(hex: string): Theme {
  const n = parseInt(hex.slice(1), 16);
  const rgb = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  const mix = (t: number, to: number) => '#' + rgb.map((v) => Math.round(v + (to - v) * t).toString(16).padStart(2, '0')).join('');
  return { id: 'custom', name: 'Custom', a1: hex, a2: mix(0.35, 255), a3: mix(0.35, 0) };
}
