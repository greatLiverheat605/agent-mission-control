import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Command, Search } from "@mission-control/ui";
import { useFocusReturn } from "./useFocusReturn";
import { useLocale } from "../i18n/LocaleProvider";
import "./interaction.css";

export type MissionCommand = {
  id: string;
  label: string;
  kind: "mission" | "route" | "evidence" | "approval" | "view" | "command";
  keywords: string[];
  enabled?: boolean;
  dangerous?: boolean;
  run: () => void;
};

export function CommandPalette({ open, commands, onClose, onRequestConfirmation }: { open: boolean; commands: MissionCommand[]; onClose: () => void; onRequestConfirmation: (command: MissionCommand) => void }) {
  const { t } = useLocale();
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const dialog = useRef<HTMLElement>(null);
  useFocusReturn(open);
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return commands.filter((command) => command.enabled !== false && (!needle || `${command.label} ${command.keywords.join(" ")}`.toLowerCase().includes(needle)));
  }, [commands, query]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActiveIndex(0);
    queueMicrotask(() => input.current?.focus());
  }, [open]);

  if (!open) return null;
  const activate = (command: MissionCommand | undefined) => {
    if (!command) return;
    if (command.dangerous) onRequestConfirmation(command);
    else command.run();
    onClose();
  };
  const trapFocus = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(dialog.current?.querySelectorAll<HTMLElement>("input, button:not([disabled])") ?? []);
    if (!focusable.length) return;
    const index = focusable.indexOf(document.activeElement as HTMLElement);
    const next = event.shiftKey ? (index <= 0 ? focusable.length - 1 : index - 1) : (index + 1) % focusable.length;
    event.preventDefault();
    focusable[next].focus();
  };
  return <div className="command-palette-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}>
    <section ref={dialog} className="command-palette" role="dialog" aria-modal="true" aria-labelledby="command-palette-title" onKeyDown={trapFocus}>
      <header><Command aria-hidden="true" size={19} /><h2 id="command-palette-title">{t("palette.title")}</h2></header>
      <label className="command-search"><Search aria-hidden="true" size={18} /><span className="sr-only">{t("palette.search")}</span><input
        ref={input}
        role="combobox"
        aria-label={t("palette.search")}
        aria-controls="mission-command-list"
        aria-expanded="true"
        aria-activedescendant={visible[activeIndex] ? `command-${visible[activeIndex].id}` : undefined}
        value={query}
        onChange={(event) => { setQuery(event.target.value); setActiveIndex(0); }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex((index) => Math.min(visible.length - 1, index + 1)); }
          else if (event.key === "ArrowUp") { event.preventDefault(); setActiveIndex((index) => Math.max(0, index - 1)); }
          else if (event.key === "Enter") { event.preventDefault(); activate(visible[activeIndex]); }
        }}
      /></label>
      <div id="mission-command-list" className="command-results" role="listbox" aria-label={t("palette.available")}>
        {visible.length ? visible.map((command, index) => <button
          id={`command-${command.id}`}
          key={command.id}
          type="button"
          role="option"
          aria-selected={index === activeIndex}
          onMouseEnter={() => setActiveIndex(index)}
          onClick={() => activate(command)}
        ><span>{command.label}</span><small>{command.kind}</small></button>) : <p>{t("palette.empty")}</p>}
      </div>
    </section>
  </div>;
}
