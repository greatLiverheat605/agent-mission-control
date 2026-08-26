import type { EN_MESSAGES } from "./catalogs/en-US";

export const LOCALES = ["en-US", "zh-CN"] as const;
export type Locale = typeof LOCALES[number];
export type MessageKey = keyof typeof EN_MESSAGES;
export type MessageCatalog = Record<MessageKey, string>;
export type MessageValues = Record<string, string | number>;

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && (LOCALES as readonly string[]).includes(value);
}
