// Minimal stand-in: drag-and-drop of OS files only works with the real package.
export function getCurrentWebview() {
  return {
    async onDragDropEvent(_cb: (e: { payload: { type: string; paths?: string[] } }) => void) {
      return () => {};
    },
  };
}
