// Stand-in for @tauri-apps/api/window outside the desktop app (browser tests).
export function getCurrentWindow() {
  const noop = async () => {};
  return { minimize: noop, toggleMaximize: noop, close: noop, isMaximized: async () => false, onResized: async () => () => {}, startDragging: noop };
}
