import { useEffect } from "react";

export function isCommandPaletteShortcut(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey">): boolean {
  return event.key.toLowerCase() === "k" && (event.ctrlKey || event.metaKey);
}

export function useMissionKeyboard(openPalette: () => void): void {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isCommandPaletteShortcut(event)) return;
      event.preventDefault();
      openPalette();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [openPalette]);
}
