import { useEffect, useState } from 'react';

/**
 * The order of project groups, set by dragging a group (dashboard or
 * sidebar). Shared by both: changing it in one updates the other. Groups not
 * placed yet go after the placed ones, A→Z. Kept on this computer, like the
 * order of cards.
 */
const KEY = 'pv.groupOrder';
const EVT = 'pv-group-order';

export function getGroupOrder(): string[] {
  try { return JSON.parse(localStorage.getItem(KEY) || '[]'); } catch { return []; }
}

export function setGroupOrder(o: string[]) {
  try { localStorage.setItem(KEY, JSON.stringify(o)); } catch { /* ignore */ }
  window.dispatchEvent(new Event(EVT));
}

/** Group names in your order; ones you haven't placed after them, A→Z. */
export function sortGroups(names: string[], order: string[]): string[] {
  const at = (g: string) => { const i = order.indexOf(g); return i < 0 ? Infinity : i; };
  return [...names].sort((a, b) => {
    const d = at(a) - at(b);
    if (d) return Number.isFinite(d) ? d : at(a) === Infinity ? 1 : -1;
    return a.localeCompare(b, undefined, { sensitivity: 'base' });
  });
}

/** Moves `g` before (or after) `target`, given the groups as currently shown. */
export function moveGroup(shown: string[], g: string, target: string, after: boolean) {
  if (g === target) return;
  const rest = shown.filter((x) => x !== g);
  const i = rest.indexOf(target);
  if (i < 0) return;
  rest.splice(i + (after ? 1 : 0), 0, g);
  // keep the places of groups that aren't shown right now (e.g. filtered out)
  setGroupOrder([...rest, ...getGroupOrder().filter((x) => !rest.includes(x))]);
}

/** A renamed group keeps its place. */
export function renameInGroupOrder(from: string, to: string) {
  const o = getGroupOrder();
  const i = o.indexOf(from);
  if (i < 0) return;
  const n = o.filter((x) => x !== to);
  n[n.indexOf(from)] = to;
  setGroupOrder(n);
}

export function useGroupOrder(): string[] {
  const [o, setO] = useState(getGroupOrder);
  useEffect(() => {
    const on = () => setO(getGroupOrder());
    window.addEventListener(EVT, on);
    window.addEventListener('storage', on);
    return () => { window.removeEventListener(EVT, on); window.removeEventListener('storage', on); };
  }, []);
  return o;
}

export const GROUP_DRAG = 'text/pv-group';
