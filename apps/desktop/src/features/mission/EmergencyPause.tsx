import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { OctagonX, Pause } from "@mission-control/ui";
import "../../interaction/interaction.css";
import { useLocale } from "../../i18n/LocaleProvider";

export function EmergencyPause({ disabled = false, showPause = true, onPause, onForceTerminate }: { disabled?: boolean; showPause?: boolean; onPause: () => void; onForceTerminate?: () => void }) {
  const { t } = useLocale();
  const [confirming, setConfirming] = useState(false);
  const [coverOpen, setCoverOpen] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirming) queueMicrotask(() => cancelRef.current?.focus());
  }, [confirming]);
  const trapConfirmFocus = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setConfirming(false);
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = [cancelRef.current, confirmRef.current].filter((node): node is HTMLButtonElement => Boolean(node));
    if (!focusable.length) return;
    const index = focusable.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.shiftKey ? (index <= 0 ? focusable.length - 1 : index - 1) : (index + 1) % focusable.length;
    event.preventDefault();
    focusable[next].focus();
  };
  return <div className="emergency-controls" data-has-pause={showPause}>
    <span className="emergency-label">{t("emergency.force")}</span>
    {showPause && <button className="emergency-pause" type="button" disabled={disabled} onClick={onPause} aria-label={t("emergency.pauseAria")}><Pause aria-hidden="true" size={18} />{t("emergency.pause")}</button>}
    <div className="emergency-guard" data-open={coverOpen}>
      <button className="force-terminate" type="button" disabled={!onForceTerminate || !coverOpen} aria-label={t("emergency.force")} title={!onForceTerminate ? t("emergency.unavailable") : t("emergency.force")} onClick={() => setConfirming(true)}><OctagonX aria-hidden="true" size={18} /><span>{t("emergency.force")}</span></button>
      <button className="safety-cover" type="button" aria-expanded={coverOpen} aria-label={t("emergency.cover")} onClick={() => setCoverOpen((open) => !open)}><span>▲</span>{t("emergency.cover")}</button>
    </div>
    {confirming && <div className="force-confirm-backdrop" role="presentation"><section className="force-confirm" role="dialog" aria-modal="true" aria-labelledby="force-confirm-title" onKeyDown={trapConfirmFocus}>
      <header><OctagonX aria-hidden="true" size={20} /><h2 id="force-confirm-title">{t("emergency.confirmTitle")}</h2></header>
      <p>{t("emergency.confirmBody")}</p>
      <div className="force-confirm-actions"><button ref={cancelRef} type="button" onClick={() => setConfirming(false)}>{t("emergency.keepRunning")}</button><button ref={confirmRef} type="button" className="danger-command" onClick={() => { setConfirming(false); onForceTerminate?.(); }}>{t("emergency.confirm")}</button></div>
    </section></div>}
  </div>;
}
