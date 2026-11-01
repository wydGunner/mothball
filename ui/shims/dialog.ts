import { invoke } from './core';
export async function open(options: Record<string, unknown> = {}): Promise<string | string[] | null> {
  return invoke('plugin:dialog|open', { options });
}
