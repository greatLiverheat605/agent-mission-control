import { useEffect, useRef } from "react";

export function useFocusReturn(open: boolean): void {
  const returnTarget = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (open) {
      returnTarget.current ??= document.activeElement instanceof HTMLElement ? document.activeElement : null;
      return;
    }
    const target = returnTarget.current;
    returnTarget.current = null;
    target?.focus();
  }, [open]);

  useEffect(() => () => returnTarget.current?.focus(), []);
}
