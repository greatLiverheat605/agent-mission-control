import { useEffect, useMemo, useRef, useState } from "react";
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
  return <div className="command-palette-backdrop" role="presentation" onMouseDown={(event) => { if (event.currentTarget === event.target) onClose(); }}>
    <section className="command-palette" role="dialog" aria-modal="true" aria-labelledby="command-palette-title">
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
          if (event.key === "Escape") onClose();
          else if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex((index) => Math.min(visible.length - 1, index + 1)); }
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
