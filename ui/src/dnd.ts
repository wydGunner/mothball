/**
 * Dragging things around inside the app (cards, groups, panels, tasks,
 * notes), driven by the mouse instead of the browser's built-in drag and drop.
 *
 * Why: in the desktop app on Windows, Tauri takes over the system drag and
 * drop so files can be dragged in from Explorer (with their real paths). That
 * leaves drags *inside* the page showing the "not allowed" cursor and never
 * dropping. This module watches the mouse on any `draggable="true"` element
 * and sends the same events the browser would (dragstart, dragenter,
 * dragover, dragleave, drop, dragend) with a working DataTransfer, so every
 * existing onDragStart/onDragOver/onDrop handler keeps working unchanged.
 * Files dragged in from outside the app still go through Tauri as before.
 */

const THRESHOLD = 5; // px the mouse must move before a press becomes a drag

export function installPointerDnd() {
  let src: HTMLElement | null = null;
  let sx = 0;
  let sy = 0;
  let pointerId = -1;
  let dragging = false;
  let dt: DataTransfer | null = null;
  let over: Element | null = null;
  let accepted = false;
  let ghost: HTMLElement | null = null;

  const fire = (target: Element, type: string, x: number, y: number) => {
    const ev = new DragEvent(type, { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, dataTransfer: dt });
    target.dispatchEvent(ev);
    return ev.defaultPrevented;
  };

  const cleanup = () => {
    if (src) src.classList.remove('pv-drag-src');
    document.documentElement.classList.remove('pv-dragging', 'pv-drag-ok');
    ghost?.remove();
    ghost = null;
    src = null;
    dragging = false;
    dt = null;
    over = null;
    accepted = false;
    pointerId = -1;
  };

  // a click right after a drag ends would "open" what was dragged: swallow it
  const swallowNextClick = () => {
    const stop = (e: Event) => { e.stopPropagation(); e.preventDefault(); };
    window.addEventListener('click', stop, { capture: true, once: true });
    setTimeout(() => window.removeEventListener('click', stop, { capture: true }), 0);
  };

  const makeGhost = (el: HTMLElement, x: number, y: number) => {
    const r = el.getBoundingClientRect();
    const g = el.cloneNode(true) as HTMLElement;
    g.classList.add('pv-drag-ghost');
    g.style.width = `${Math.min(r.width, 360)}px`;
    g.style.left = `${x}px`;
    g.style.top = `${y}px`;
    document.body.appendChild(g);
    return g;
  };

  window.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || !e.isPrimary) return;
    const t = e.target as HTMLElement | null;
    if (!t || t.closest('input, textarea, select, [contenteditable="true"], .monaco-editor')) return;
    const el = t.closest('[draggable="true"]') as HTMLElement | null;
    if (!el) return;
    src = el;
    sx = e.clientX;
    sy = e.clientY;
    pointerId = e.pointerId;
  }, true);

  window.addEventListener('pointermove', (e) => {
    if (!src || e.pointerId !== pointerId) return;
    const x = e.clientX;
    const y = e.clientY;
    if (!dragging) {
      if (Math.abs(x - sx) < THRESHOLD && Math.abs(y - sy) < THRESHOLD) return;
      dt = new DataTransfer();
      dt.effectAllowed = 'move';
      if (fire(src, 'dragstart', sx, sy)) {
        // the handler refused the drag (preventDefault): not a drag after all
        src = null;
        dt = null;
        return;
      }
      dragging = true;
      src.classList.add('pv-drag-src');
      document.documentElement.classList.add('pv-dragging');
      window.getSelection()?.removeAllRanges();
      ghost = makeGhost(src, x, y);
      // handlers often note the drag in React state on dragstart; let that
      // settle before the first dragover (the next move, or the drop)
      return;
    }
    e.preventDefault();
    if (ghost) {
      ghost.style.left = `${x + 12}px`;
      ghost.style.top = `${y + 8}px`;
    }
    // the element under the mouse (the ghost ignores the mouse, so it isn't hit)
    const t = document.elementFromPoint(x, y);
    if (t !== over) {
      if (over) fire(over, 'dragleave', x, y);
      if (t) fire(t, 'dragenter', x, y);
      over = t;
    }
    accepted = t ? fire(t, 'dragover', x, y) : false;
    document.documentElement.classList.toggle('pv-drag-ok', accepted);
  }, true);

  let finishing = false;
  const finish = (e: PointerEvent, drop: boolean) => {
    if (!src || e.pointerId !== pointerId || finishing) return;
    if (!dragging) { cleanup(); return; }
    swallowNextClick();
    finishing = true;
    const { clientX: x, clientY: y } = e;
    // after React has caught up with the drag: one last dragover where the
    // mouse was let go (so a quick drag still lands), then drop
    setTimeout(() => {
      const s = src;
      if (s) {
        if (drop) {
          const t = document.elementFromPoint(x, y);
          if (t !== over) {
            if (over) fire(over, 'dragleave', x, y);
            if (t) fire(t, 'dragenter', x, y);
            over = t;
          }
          accepted = t ? fire(t, 'dragover', x, y) : false;
        }
        if (drop && over && accepted) fire(over, 'drop', x, y);
        else if (over) fire(over, 'dragleave', x, y);
        fire(s, 'dragend', x, y);
      }
      cleanup();
      finishing = false;
    }, 0);
  };
  window.addEventListener('pointerup', (e) => finish(e, true), true);
  window.addEventListener('pointercancel', (e) => finish(e, false), true);
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !dragging || !src || finishing) return;
    e.stopPropagation();
    if (over) fire(over, 'dragleave', sx, sy);
    fire(src, 'dragend', sx, sy);
    cleanup();
  }, true);

  // The browser's own drag would start too and take the mouse away (and in
  // the desktop app it can't drop anyway): stop it for in-app draggables.
  // Our own (untrusted) events pass through.
  window.addEventListener('dragstart', (e) => {
    if (!e.isTrusted) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.('[draggable="true"]')) {
      e.preventDefault();
      e.stopPropagation();
    }
  }, true);
}
