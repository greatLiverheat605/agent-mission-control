import { describe, expect, test } from "vitest";
import { emptyMission, reduceMission, type MissionStoreState } from "./reducer";

const event = (sequence: number, kind = "agent_message") => ({ missionId: "m1", sequence, kind, payload: {} });

describe("mission read model", () => {
  test("marks sequence gaps for supervisor resync", () => {
    let state: MissionStoreState = { m1: emptyMission("m1") };
    state = reduceMission(state, { type: "event", event: event(2) });
    expect(state.m1.needsResync).toBe(true);
    expect(state.m1.lastSequence).toBe(0);
  });

  test("does not replay duplicate events", () => {
    let state: MissionStoreState = { m1: emptyMission("m1") };
    state = reduceMission(state, { type: "event", event: event(1) });
    state = reduceMission(state, { type: "event", event: event(1) });
    expect(state.m1.events).toHaveLength(1);
  });
});
