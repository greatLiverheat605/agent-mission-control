import { useEffect, useState } from "react";

export function useCssColor(token: string, fallback = "white"): string {
  const [color, setColor] = useState(fallback);
  useEffect(() => {
    const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
    if (value) setColor(value);
  }, [token]);
  return color;
}
