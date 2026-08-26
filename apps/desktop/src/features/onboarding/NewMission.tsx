import { useState, type FormEvent } from "react";
import { Route, ShieldCheck, SquareTerminal } from "@mission-control/ui";
import { LocaleSwitcher, useLocale } from "../../i18n/LocaleProvider";

export type MissionProvider = "codex" | "claude" | "opencode" | "zcode";
export type MissionDraft = { projectRoot: string; goal: string; agent: MissionProvider };
export type ProviderCapabilityView = { provider: MissionProvider; available: boolean; installState: string; version?: string; unavailableReason?: string };

export function NewMission({ onCreate, embedded = false, capabilities, ccSwitch }: { onCreate: (draft: MissionDraft) => void; embedded?: boolean; capabilities?: ProviderCapabilityView[]; ccSwitch?: { available: boolean; version?: string; reason?: string } }) {
  const { t } = useLocale();
  const [projectRoot, setProjectRoot] = useState("");
  const [goal, setGoal] = useState("");
  const [agent, setAgent] = useState<MissionDraft["agent"]>("codex");
  const submit = (event: FormEvent) => { event.preventDefault(); if (projectRoot.trim() && goal.trim()) onCreate({ projectRoot: projectRoot.trim(), goal: goal.trim(), agent }); };
  return <div className={`mission-launch-sequence${embedded ? " mission-launch-sequence--embedded" : ""}`}>
    {!embedded && <header className="preflight-beam" aria-label={t("new.preflightStatus")}><div><span>{t("new.vessel")}</span><strong>{t("beam.vesselName")}</strong></div><div><span>{t("new.flightMode")}</span><strong>{t("new.readOnlyIntake")}</strong></div><div><span>{t("new.supervisor")}</span><strong>{t("new.uplinkReady")}</strong></div><LocaleSwitcher className="preflight-language" /></header>}
    <div className="preflight-grid">
      <aside className="preflight-console" aria-label={t("new.safeguards")}><ShieldCheck aria-hidden="true" size={22} /><span className="eyebrow">{t("new.authorityBoundary")}</span><strong>{t("new.contractLock")}</strong><p>{t("new.contractNote")}</p></aside>
      <form className="mission-panel" onSubmit={submit} aria-labelledby="new-mission-title">
        <div className="mission-panel__heading"><SquareTerminal aria-hidden="true" size={22} /><div><span className="eyebrow">{t("new.preflight")}</span><h1 id="new-mission-title">{t("new.title")}</h1></div></div>
        <label>{t("new.projectFolder")}<input aria-label={t("new.projectFolder")} value={projectRoot} onChange={(event) => setProjectRoot(event.target.value)} placeholder="C:\workspace\project" /></label>
        <label>{t("new.goal")}<textarea aria-label={t("new.goal")} value={goal} onChange={(event) => setGoal(event.target.value)} placeholder={t("new.goalPlaceholder")} rows={3} /></label>
        <label>{t("new.agent")}<select aria-label={t("new.agent")} value={agent} onChange={(event) => setAgent(event.target.value as MissionDraft["agent"])}>{["codex", "claude", "opencode", "zcode"].map((provider) => { const capability = capabilities?.find((item) => item.provider === provider); const available = capability?.available ?? (provider === "codex" || provider === "claude"); return <option key={provider} value={provider} disabled={!available}>{providerLabel(provider)}{available ? "" : ` - ${t("new.unavailable")}`}</option>; })}</select></label>
        <div className="preflight-capabilities" aria-label={t("new.capabilities")}><span>{t("new.capabilities")}</span><strong>{capabilitySummary(capabilities, ccSwitch)}</strong></div>
        <button type="submit" aria-label={t("new.review")}><Route aria-hidden="true" size={18} />{t("new.reviewVisible")}</button>
      </form>
      <aside className="preflight-console" aria-label={t("new.launchSequence")}><Route aria-hidden="true" size={22} /><span className="eyebrow">{t("new.launchSequence")}</span><ol><li><span>01</span>{t("new.projectLock")}</li><li><span>02</span>{t("new.contractReview")}</li><li><span>03</span>{t("new.routeIgnition")}</li></ol></aside>
    </div>
    {!embedded && <footer className="preflight-helm"><span>{t("beam.control")}</span><strong>{t("new.awaiting")}</strong><span>{t("new.safeHold")}</span></footer>}
  </div>;
}

function providerLabel(provider: string) {
  return provider === "opencode" ? "OpenCode" : provider === "zcode" ? "ZCode" : provider === "claude" ? "Claude" : "Codex";
}

function capabilitySummary(capabilities?: ProviderCapabilityView[], ccSwitch?: { available: boolean; version?: string }) {
  const available = capabilities?.filter((item) => item.available).length ?? 2;
  const cc = ccSwitch?.available ? "CC Switch ready" : "CC Switch unavailable";
  return `${available}/4 providers · ${cc}`;
}
