import type { RouteState } from "@mission-control/mission-store";
import { Activity, GitBranch } from "@mission-control/ui";
import { useLocale } from "../../i18n/LocaleProvider";

export type ProjectMissionItem = {
  id: string;
  label: string;
  routeState: RouteState;
  action: string;
};

export function ProjectOrbit({ missions, selectedId, onSelect }: { missions: ProjectMissionItem[]; selectedId: string; onSelect?: (missionId: string) => void }) {
  const { t } = useLocale();
  return <section className="orbit-panel" aria-labelledby="project-orbit-title">
    <header className="panel-heading">
      <span className="panel-kicker">{t("panel.projectOrbit")}</span>
      <h2 id="project-orbit-title">{t("panel.missions")}</h2>
    </header>
    <div className="project-mission-list">
      {missions.map((mission) => {
        const active = mission.id === selectedId;
        return <button key={mission.id} type="button" className="project-mission" aria-current={active ? "page" : undefined} onClick={() => onSelect?.(mission.id)}>
          {active ? <Activity aria-hidden="true" size={17} /> : <GitBranch aria-hidden="true" size={17} />}
          <span className="project-mission__body"><span className="project-mission__head"><strong>{mission.label}</strong><code>{mission.id.slice(-4)}</code></span><small><i data-route-state={mission.routeState}>{t(`status.${mission.routeState}`)}</i><span>{mission.action}</span></small></span>
        </button>;
      })}
    </div>
  </section>;
}
