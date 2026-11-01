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

export function renderMarkdown(md: string): string {
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
