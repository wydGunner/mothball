import { invoke } from './core';
export async function openPath(path: string): Promise<void> {
  return invoke('plugin:opener|open_path', { path });
}
export async function revealItemInDir(path: string): Promise<void> {
  return invoke('plugin:opener|reveal_item_in_dir', { paths: [path] });
}
