// Offline-build stand-in for @tauri-apps/api/core (same IPC call underneath).
export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const t = (window as any).__TAURI_INTERNALS__;
  if (!t) throw new Error('Not running inside Tauri');
  return t.invoke(cmd, args);
}
