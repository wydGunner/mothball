import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { LuChevronDown, LuChevronRight, LuGripVertical } from 'react-icons/lu';
import { cx } from '../format';
import { useContextMenu } from './common';

/** A block on a page that can be dragged to another position/column and collapsed. */
export interface PanelDef {
  id: string;
  title: ReactNode;
  /** Buttons on the right of the header (stay visible when collapsed). */
  actions?: ReactNode;
  /** Small text after the title, e.g. a count. */
  meta?: ReactNode;
  /** Small icon before the title, in the accent colour. */
  icon?: ReactNode;
  /** One line shown in the header while the panel is collapsed. */
  summary?: ReactNode;
  className?: string;
  body: ReactNode;
  /** Not shown right now (keeps its place in the layout). */
  hidden?: boolean;
}

interface Layout { zones: Record<string, string[]>; collapsed: string[] }

function load(key: string): Layout | null {
  try { const j = JSON.parse(localStorage.getItem('pv.layout.' + key) || 'null'); return j && j.zones ? j : null; } catch { return null; }
}

/** Puts every known panel in exactly one zone: saved positions first, new panels where the defaults put them. */
function normalize(saved: Layout | null, defaults: Record<string, string[]>): Layout {
  const known = new Set(Object.values(defaults).flat());
  const zones: Record<string, string[]> = {};
  const placed = new Set<string>();
  for (const z of Object.keys(defaults)) {
    zones[z] = (saved?.zones[z] || []).filter((id) => known.has(id) && !placed.has(id) && (placed.add(id), true));
  }
  for (const [z, ids] of Object.entries(defaults)) {
    ids.forEach((id, i) => {
      if (placed.has(id)) return;
      // insert near its default neighbour
      const prev = ids.slice(0, i).reverse().find((x) => zones[z].includes(x));
      const at = prev ? zones[z].indexOf(prev) + 1 : 0;
      zones[z].splice(at, 0, id);
      placed.add(id);
    });
  }
  return { zones, collapsed: (saved?.collapsed || []).filter((id) => known.has(id)) };
}

/**
 * Lays out panels in named zones (columns or areas). Drag a panel's header to
 * move it (also into another zone), click the chevron to collapse it. The
 * layout is remembered per board. Right-click a header for keyboard-friendly
 * Move up / Move down / Reset layout.
 */
export function PanelBoard({ storageKey, defaults, panels, zoneClass, before }: {
  storageKey: string;
  defaults: Record<string, string[]>;
  panels: PanelDef[];
  zoneClass: Record<string, string>;
  /** Rendered at the start of a zone, not movable (e.g. a page title). */
  before?: Record<string, ReactNode>;
}) {
  const defaultsKey = JSON.stringify(defaults);
  const [layout, setLayout] = useState<Layout>(() => normalize(load(storageKey), defaults));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { setLayout((l) => normalize(l, defaults)); }, [defaultsKey]);
  const save = (l: Layout) => { setLayout(l); try { localStorage.setItem('pv.layout.' + storageKey, JSON.stringify(l)); } catch { /* ignore */ } };
  const byId = useMemo(() => Object.fromEntries(panels.map((p) => [p.id, p])), [panels]);
  const [drag, setDrag] = useState<string | null>(null);
  const [drop, setDrop] = useState<{ zone: string; index: number } | null>(null);
  const menu = useContextMenu();

  const move = (id: string, zone: string, index: number) => {
    const zones = Object.fromEntries(Object.entries(layout.zones).map(([z, ids]) => [z, [...ids]]));
    let from = '', fromIdx = -1;
    for (const [z, ids] of Object.entries(zones)) { const i = ids.indexOf(id); if (i >= 0) { from = z; fromIdx = i; ids.splice(i, 1); } }
    if (from === zone && fromIdx < index) index--;
    zones[zone].splice(Math.max(0, Math.min(index, zones[zone].length)), 0, id);
    save({ ...layout, zones });
  };
  const toggle = (id: string) => save({ ...layout, collapsed: layout.collapsed.includes(id) ? layout.collapsed.filter((x) => x !== id) : [...layout.collapsed, id] });
  const reset = () => { try { localStorage.removeItem('pv.layout.' + storageKey); } catch { /* ignore */ } setLayout(normalize(null, defaults)); };
  const isCustom = JSON.stringify(layout) !== JSON.stringify(normalize(null, defaults));

  // Where a dragged panel would land, from the pointer position anywhere on the
  // page: the zone under (or nearest to) the pointer, then the slot in reading
  // order. Empty space below a zone means "at the end of it".
  const zoneEls = useRef<Record<string, HTMLDivElement | null>>({});
  const target = (x: number, y: number): { zone: string; index: number } | null => {
    let best: { zone: string; dist: number } | null = null;
    for (const [zone, el] of Object.entries(zoneEls.current)) {
      if (!el) continue;
      const r = el.getBoundingClientRect();
      const dx = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
      const dy = y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
      const dist = dx * 4 + dy; // prefer the column the pointer is in
      if (!best || dist < best.dist) best = { zone, dist };
    }
    if (!best) return null;
    const el = zoneEls.current[best.zone]!;
    const ids = layout.zones[best.zone];
    const panelsEls = [...el.querySelectorAll(':scope > [data-panel]')] as HTMLElement[];
    const rects = panelsEls.map((p) => ({ id: p.dataset.panel!, r: p.getBoundingClientRect() }));
    const multiCol = rects.some((a, i) => rects.some((b, j) => i !== j && Math.abs(a.r.top - b.r.top) < 8));
    for (const { id, r } of rects) {
      if (id === drag) continue;
      const before = y < r.top || (y <= r.bottom && (multiCol ? x < r.left + r.width / 2 && y < r.bottom : y < r.top + r.height / 2));
      if (before) return { zone: best.zone, index: ids.indexOf(id) };
    }
    return { zone: best.zone, index: ids.length };
  };
  useEffect(() => {
    if (!drag) return;
    const over = (e: DragEvent) => {
      if (!e.dataTransfer?.types.includes('text/pv-panel')) return;
      e.preventDefault();
      const t = target(e.clientX, e.clientY);
      setDrop((d) => (t && (!d || d.zone !== t.zone || d.index !== t.index) ? t : d));
    };
    const dropped = (e: DragEvent) => {
      if (!e.dataTransfer?.types.includes('text/pv-panel')) return;
      e.preventDefault();
      const t = target(e.clientX, e.clientY);
      if (t) move(drag, t.zone, t.index);
      setDrag(null); setDrop(null);
    };
    window.addEventListener('dragover', over);
    window.addEventListener('drop', dropped);
    return () => { window.removeEventListener('dragover', over); window.removeEventListener('drop', dropped); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag, layout]);

  return (
    <>
      {Object.keys(layout.zones).map((zone) => {
        const ids = layout.zones[zone];
        const visible = ids.map((id, i) => ({ id, i })).filter(({ id }) => byId[id] && !byId[id].hidden);
        return (
          <div key={zone} ref={(el) => { zoneEls.current[zone] = el; }} className={cx(zoneClass[zone], 'pzone', drag && 'dragging', drag && drop?.zone === zone && drop.index >= ids.length && !visible.some((v) => v.id !== drag) && 'drop-end')}>
            {before?.[zone]}
            {visible.map(({ id, i }) => {
              const p = byId[id];
              const collapsed = layout.collapsed.includes(id);
              const line = drop && drop.zone === zone ? (drop.index === i ? 'drop-before' : drop.index === i + 1 && !visible.some((v) => v.i === i + 1) ? 'drop-after' : '') : '';
              return (
                <section key={id} className={cx('panel', p.className, collapsed && 'collapsed', drag === id && 'is-dragged', line)} data-panel={id}>
                  <div className="panel-h" draggable
                    onDragStart={(e) => { e.dataTransfer.setData('text/pv-panel', id); e.dataTransfer.effectAllowed = 'move'; setDrag(id); }}
                    onDragEnd={() => { setDrag(null); setDrop(null); }}
                    onContextMenu={(e) => {
                      const all = Object.entries(layout.zones);
                      const zi = all.findIndex(([z]) => z === zone);
                      menu(e, [
                        { label: collapsed ? 'Expand' : 'Collapse', onClick: () => toggle(id) },
                        { label: 'Move up', disabled: i === 0, onClick: () => move(id, zone, i - 1) },
                        { label: 'Move down', disabled: i >= ids.length - 1, onClick: () => move(id, zone, i + 2) },
                        ...all.filter(([z]) => z !== zone).map(([z]) => ({ label: `Move to the ${zoneLabel(z, zi, all.findIndex(([x]) => x === z))}`, onClick: () => move(id, z, layout.zones[z].length) })),
                        { sep: true, label: '' },
                        { label: 'Reset layout', disabled: !isCustom, onClick: reset },
                      ]);
                    }}>
                    <LuGripVertical className="grip" aria-hidden title="Drag to move" />
                    <button className="panel-t" aria-expanded={!collapsed} onClick={() => toggle(id)} title={collapsed ? 'Expand' : 'Collapse'}>
                      {collapsed ? <LuChevronRight /> : <LuChevronDown />}{p.icon && <span className="panel-icon">{p.icon}</span>}<h2>{p.title}</h2>{p.meta && <span className="panel-meta">{p.meta}</span>}
                    </button>
                    {p.actions && <div className="panel-actions" onMouseDown={(e) => e.stopPropagation()}>{p.actions}</div>}
                  </div>
                  {collapsed && p.summary && <button className="panel-summary" onClick={() => toggle(id)} title="Expand">{p.summary}</button>}
                  {!collapsed && <div className="panel-b">{p.body}</div>}
                </section>
              );
            })}
          </div>
        );
      })}
    </>
  );
}

const zoneLabel = (z: string, from: number, to: number) => {
  const names: Record<string, string> = { main: 'main column', rail: 'side column', top: 'top row', bottom: 'full width area', left: 'left column', right: 'right column' };
  return names[z] || (to < from ? 'previous area' : 'next area');
};
