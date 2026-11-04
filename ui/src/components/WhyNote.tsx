import { useEffect, useRef, useState } from 'react';
import { LuMessageSquarePlus, LuPencil } from 'react-icons/lu';
import { useToast } from './common';

/**
 * "Why": your own note on an activity entry or a file change, so history
 * says why something changed, not just what. Shows the note under the entry;
 * without one, an "Add why" button that appears when you hover the row (or
 * focus it with Tab). Click to write; Enter saves, Escape cancels, an empty
 * note removes it.
 */
export function WhyNote({ note, onSave, placeholder = 'Why did this change? e.g. LLM fix for the back button overlap' }: { note?: string; onSave: (note: string) => Promise<unknown>; placeholder?: string }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(note || '');
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const toast = useToast();
  useEffect(() => { if (!editing) setText(note || ''); }, [note, editing]);
  useEffect(() => { if (editing) input.current?.focus(); }, [editing]);
  const save = async () => {
    if (saving) return;
    if (text.trim() === (note || '').trim()) { setEditing(false); return; }
    setSaving(true);
    try { await onSave(text.trim()); setEditing(false); } catch (e: any) { toast(e?.message || String(e), 'error'); } finally { setSaving(false); }
  };
  if (editing) return (
    <div className="why-edit" onClick={(e) => e.stopPropagation()}>
      <input ref={input} className="input sm" value={text} maxLength={2000} placeholder={placeholder} aria-label="Why it changed"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); save(); } else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setEditing(false); } }}
        onBlur={save} disabled={saving} />
    </div>
  );
  if (note) return (
    <button className="why-note" title="Why it changed — click to edit" onClick={(e) => { e.stopPropagation(); setEditing(true); }}>
      <span className="why-label">Why</span><span className="why-text">{note}</span><LuPencil className="why-pen" />
    </button>
  );
  return <button className="why-add" title="Add a note saying why this changed" onClick={(e) => { e.stopPropagation(); setEditing(true); }}><LuMessageSquarePlus /> Add why</button>;
}
