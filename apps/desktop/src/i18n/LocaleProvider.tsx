import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Languages } from "@mission-control/ui";
import { EN_MESSAGES } from "./catalogs/en-US";
import { ZH_MESSAGES } from "./catalogs/zh-CN";
import { isLocale, type Locale, type MessageCatalog, type MessageKey, type MessageValues } from "./types";

export const LOCALE_STORAGE_KEY = "mission-control.locale.v1";

const CATALOGS: Record<Locale, MessageCatalog> = { "en-US": EN_MESSAGES, "zh-CN": ZH_MESSAGES };

type LocaleContextValue = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: MessageKey, values?: MessageValues) => string;
  number: (value: number, options?: Intl.NumberFormatOptions) => string;
  dateTime: (value: string | number | Date, options?: Intl.DateTimeFormatOptions) => string;
};

const LocaleContext = createContext<LocaleContextValue>({
  locale: "en-US",
  setLocale: () => undefined,
  t: (key, values) => interpolate(EN_MESSAGES[key], values),
  number: (value, options) => new Intl.NumberFormat("en-US", options).format(value),
  dateTime: (value, options) => new Intl.DateTimeFormat("en-US", options).format(new Date(value)),
});

export function resolveInitialLocale(storage: Pick<Storage, "getItem"> | null = typeof localStorage === "undefined" ? null : localStorage, language = typeof navigator === "undefined" ? "en-US" : navigator.language): Locale {
  const stored = storage?.getItem(LOCALE_STORAGE_KEY);
  if (isLocale(stored)) return stored;
  return language.toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
}

export function LocaleProvider({ children, initialLocale }: { children: ReactNode; initialLocale?: Locale }) {
  const [locale, setLocaleState] = useState<Locale>(() => initialLocale ?? resolveInitialLocale());
  useEffect(() => { document.documentElement.lang = locale; }, [locale]);
  const value = useMemo<LocaleContextValue>(() => ({
    locale,
    setLocale: (next) => {
      setLocaleState(next);
      try { localStorage.setItem(LOCALE_STORAGE_KEY, next); } catch { /* Storage may be unavailable in hardened webviews. */ }
      document.documentElement.lang = next;
    },
    t: (key, values) => interpolate(CATALOGS[locale][key], values),
    number: (input, options) => new Intl.NumberFormat(locale, options).format(input),
    dateTime: (input, options) => new Intl.DateTimeFormat(locale, options).format(new Date(input)),
  }), [locale]);
  return <LocaleContext.Provider value={value}>{children}</LocaleContext.Provider>;
}

export function useLocale() {
  return useContext(LocaleContext);
}

export function LocaleSwitcher({ className }: { className?: string }) {
  const { locale, setLocale, t } = useLocale();
  const next = locale === "en-US" ? "zh-CN" : "en-US";
  return <button className={className} type="button" aria-label={t("locale.toggle")} title={t("locale.toggle")} onClick={() => setLocale(next)}><Languages aria-hidden="true" size={17} /><span>{t(`locale.current.${locale}`)}</span></button>;
}

function interpolate(message: string, values?: MessageValues) {
  if (!values) return message;
  return message.replace(/\{([^}]+)\}/g, (match, key: string) => values[key] === undefined ? match : String(values[key]));
}
