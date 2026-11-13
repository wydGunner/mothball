// Markdown → sanitized HTML. Supports wiki-style internal links:
//   [[Note title]]  [[file:path/to/file.gsc]]  [[task:Title]]
import { marked } from 'marked';

marked.setOptions({ gfm: true, breaks: true });

const ALLOWED = new Set([
  'P', 'BR', 'HR', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'STRONG', 'EM', 'DEL', 'CODE', 'PRE', 'BLOCKQUOTE', 'UL', 'OL', 'LI',
  'A', 'TABLE', 'THEAD', 'TBODY', 'TR', 'TH', 'TD', 'INPUT', 'SPAN', 'IMG', 'SUP', 'SUB', 'KBD',
]);
const ATTRS: Record<string, string[]> = { A: ['href', 'title'], IMG: ['src', 'alt', 'title'], INPUT: ['type', 'checked', 'disabled'], TH: ['align'], TD: ['align'], SPAN: ['class'], CODE: ['class'] };

function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function sanitize(html: string): string {
  const doc = new DOMParser().parseFromString(`<div>${html}</div>`, 'text/html');
  const walk = (el: Element) => {
    for (const child of Array.from(el.children)) {
      if (!ALLOWED.has(child.tagName)) {
        // keep text content of unknown elements, drop the element itself
        child.replaceWith(doc.createTextNode(child.textContent || ''));
        continue;
      }
      for (const a of Array.from(child.attributes)) {
        const ok = (ATTRS[child.tagName] || []).includes(a.name);
        const v = a.value.trim().toLowerCase();
        const badUrl = (a.name === 'href' || a.name === 'src') && !(v.startsWith('#') || v.startsWith('https:') || v.startsWith('http:') || v.startsWith('mailto:') || v.startsWith('data:image/'));
        if (!ok || badUrl) child.removeAttribute(a.name);
      }
      if (child.tagName === 'INPUT' && child.getAttribute('type') !== 'checkbox') child.remove();
      walk(child);
    }
  };
  const root = doc.body.firstElementChild!;
  walk(root);
  return root.innerHTML;
}

const TAGS = 'p|br|hr|h[1-6]|strong|em|del|code|pre|blockquote|ul|ol|li|a|table|thead|tbody|tr|th|td|input|span|img|sup|sub|kbd|b|i|s|u';
const KNOWN_TAG = new RegExp(`^</?(${TAGS})(\\s[^>]*)?/?>$`, 'i');

/** `<name>`-style placeholders aren't HTML: show them as written (outside code). */
function escapeUnknownTags(line: string): string {
  return line.split(/(`+[^`]*`+)/).map((part, i) => (i % 2 ? part : part.replace(/<\/?[A-Za-z][^<>\n]*>/g, (t) => (KNOWN_TAG.test(t) || /^<(https?:|mailto:)/i.test(t) ? t : t.replace(/</g, '&lt;').replace(/>/g, '&gt;'))))).join('');
}

/** A line laid out in columns with spaces ("!fps <n>      your own FPS"). */
const columnar = (l: string) => /\S {3,}\S/.test(l.trim()) && !l.includes('|');
/** A line of only = - _ * ~ (10 or more): a divider someone typed. */
const divider = (l: string) => /^\s*([=_*~-])\1{9,}\s*$/.test(l) || /^\s*(=\s*){10,}$/.test(l);

/**
 * Notes are often plain text rather than Markdown. Before rendering: typed
 * dividers become rules, space-aligned blocks become monospace boxes (so the
 * columns survive), and <placeholders> stay visible.
 */
export function prepPlainText(md: string): string {
  const lines = md.replace(/\r\n?/g, '\n').split('\n');
  const out: string[] = [];
  let fence: string | null = null;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    const f = /^\s*(```+|~~~+)/.exec(l);
    if (fence) { out.push(l); if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null; continue; }
    if (f) { fence = f[1]; out.push(l); continue; }
    if (divider(l)) {
      // ====== / TITLE / ======  → a section heading
      if (i + 2 < lines.length && lines[i + 1].trim() && !divider(lines[i + 1]) && divider(lines[i + 2])) {
        out.push('', '## ' + escapeUnknownTags(lines[i + 1].trim().replace(/\s{2,}/g, ' ')), '');
        i += 2;
        continue;
      }
      // "Title" with ===== under it is already a Markdown heading
      if (i > 0 && lines[i - 1].trim() && !divider(lines[i - 1]) && (i + 1 >= lines.length || !lines[i + 1].trim())) { out.push(l); continue; }
      out.push('', '---', '');
      continue;
    }
    // a short label right above a column block ("Anyone", "Admin and up") → subheading
    if (l.trim() && l.trim().length <= 40 && !/[.:;,]$/.test(l.trim()) && !columnar(l) && i + 2 < lines.length && columnar(lines[i + 1]) && columnar(lines[i + 2]) && (i === 0 || !lines[i - 1].trim() || divider(lines[i - 1]))) {
      out.push('', '#### ' + escapeUnknownTags(l.trim()), '');
      continue;
    }
    // a run of space-aligned lines (and their indented continuations)
    if (columnar(l)) {
      let j = i + 1;
      const block = [l];
      let cols = 1;
      while (j < lines.length && lines[j].trim() && !divider(lines[j]) && (columnar(lines[j]) || /^\s{6,}\S/.test(lines[j]))) {
        if (columnar(lines[j])) cols++;
        block.push(lines[j]); j++;
      }
      if (cols >= 2) {
        const ind = Math.min(...block.map((b) => b.match(/^\s*/)![0].length));
        out.push('', '```text', ...block.map((b) => b.slice(ind)), '```', '');
        i = j - 1;
        continue;
      }
    }
    // 4+ spaces of indent is already a code block in Markdown: shown as written
    out.push(/^( {4,}|\t)/.test(l) ? l : escapeUnknownTags(l));
  }
  return out.join('\n');
}

export function renderMarkdown(md: string): string {
  md = prepPlainText(md);
  const withLinks = md.replace(/\[\[([^\]\n]{1,200})\]\]/g, (_m, inner: string) => {
    const t = inner.trim();
    let kind = 'note', target = t;
    const m = /^(file|note|task):(.*)$/i.exec(t);
    if (m) { kind = m[1].toLowerCase(); target = m[2].trim(); }
    return `<a href="#pv-${kind}:${encodeURIComponent(target)}">${escapeHtml(target)}</a>`;
  });
  return sanitize(marked.parse(withLinks, { async: false }) as string);
}

/** Parses an internal-link href produced above. */
export function parseInternalHref(href: string): { kind: 'note' | 'file' | 'task'; target: string } | null {
  const m = /#pv-(note|file|task):(.*)$/.exec(href);
  return m ? { kind: m[1] as any, target: decodeURIComponent(m[2]) } : null;
}
