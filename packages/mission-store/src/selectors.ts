import type { MissionReadModel, MissionStoreState } from "./reducer";

export function selectMission(state: MissionStoreState, missionId: string): MissionReadModel | undefined { return state[missionId]; }
export function selectLatestEvents(state: MissionStoreState, missionId: string, count = 20) { return (state[missionId]?.events ?? []).slice(-count); }
export function selectNeedsResync(state: MissionStoreState, missionId: string) { return state[missionId]?.needsResync ?? false; }
export function selectMissionSummary(state: MissionStoreState, missionId: string) {
  const mission = state[missionId];
  return mission ? { phase: mission.phase, status: mission.status, action: mission.currentAction, reason: mission.reason, sequence: mission.lastSequence } : null;
}
