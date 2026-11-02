// The project type picker: built-in types, subtypes grouped under their type, and your own types.
import { typeGroups, useTypes } from '../types';

export function TypeSelect({ value, onChange, keep, id }: { value: string; onChange: (key: string) => void; keep?: string; id?: string }) {
  useTypes();
  const groups = typeGroups(keep ?? value);
  return (
    <select id={id} className="input" value={value} onChange={(e) => onChange(e.target.value)} aria-label="Project type">
      {groups.map((g) => g.label
        ? <optgroup key={g.label} label={g.label}>{g.options.map(([k, v]) => <option key={k} value={k}>{v}</option>)}</optgroup>
        : g.options.map(([k, v]) => <option key={k} value={k}>{v}</option>))}
    </select>
  );
}
