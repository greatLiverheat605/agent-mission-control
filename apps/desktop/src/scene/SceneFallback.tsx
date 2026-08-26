import type { FlightViewModel } from "@mission-control/mission-store";
import { useLocale } from "../i18n/LocaleProvider";
import "./sceneFallback.css";

export function SceneFallback({
  flight,
  onStageSelect,
}: {
  flight: FlightViewModel;
  onStageSelect?: (stageId: string) => void;
}) {
  const { t } = useLocale();
  const label = t("scene.aria", { mission: flight.mission.label, state: t(`status.${flight.primaryRoute.state}`), summary: flight.currentAction.summary, decision: flight.currentAction.nextDecision });
  const nodes = flight.stages.map((stage, index) => ({ ...stage, x: stageX(index, flight.stages.length) }));
  const agent = nodes[Math.min(flight.agentPosition.stageIndex, Math.max(0, nodes.length - 1))];

  return <div className="scene-fallback" data-route-state={flight.primaryRoute.state}>
    <div className="scene-fallback__plot" role="img" aria-label={label} data-scene-ready="fallback">
      <svg viewBox="0 0 1000 400" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
        <line className="scene-fallback__spine" x1="80" y1="200" x2="920" y2="200" />
        {flight.derivedRoutes.map((route, index) => <g key={route.id} className="scene-fallback__branch" data-state={route.state}>
          <path d={`M 500 200 L ${680 + index * 36} ${96 - index * 18}`} />
          <rect x={672 + index * 36} y={88 - index * 18} width="16" height="16" />
        </g>)}
        {nodes.map((stage) => <g key={stage.id} className="scene-fallback__node" data-stage-state={stage.state} transform={`translate(${stage.x} 200)`}>
          <circle r={stage.state === "current" ? 13 : 9} />
          <text y="38" textAnchor="middle">{t(`status.${stage.routeState}`)}</text>
        </g>)}
        {agent && <g className="scene-fallback__agent" data-testid="fallback-agent" transform={`translate(${agent.x} 200)`}>
          <path d="M 0 -17 L 12 0 L 0 17 L -12 0 Z" />
        </g>}
      </svg>
      <div className="scene-fallback__branches">
        {flight.derivedRoutes.map((route) => <span key={route.id}>{t("scene.routeSummary", { state: t(`status.${route.state}`), route: route.id })}</span>)}
      </div>
    </div>
    <div className="scene-fallback__controls" aria-label={t("scene.stages")}>
      {nodes.map((stage) => <button
        key={stage.id}
        type="button"
        data-stage-state={stage.state}
        aria-current={stage.state === "current" ? "step" : undefined}
        onClick={() => onStageSelect?.(stage.id)}
      >{t("scene.focusStage", { stage: t(`status.${stage.routeState}`) })}</button>)}
    </div>
  </div>;
}

function stageX(index: number, total: number): number {
  return total < 2 ? 500 : 80 + index * (840 / (total - 1));
}
