import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BasicMissionFlight } from "./features/mission/BasicMissionFlight";
import { NewMission, type MissionDraft } from "./features/onboarding/NewMission";
import type { MemoryDecision } from "./features/memory";
import type { ApprovalResolution } from "./features/approval/ApprovalDock";
import type { RecoveryReviewManifest } from "./features/recovery";
import type { DiagnosticPreviewData, PreviewMetricsData } from "./features/diagnostics";
import type { ExportPreviewData, StorageImpact, StorageSnapshot } from "./features/storage";
import { emptyMission, reduceMission, toFlightViewModel, type MissionEvent, type MissionReadModel } from "../../../packages/mission-store/src";
import { useLocale } from "./i18n/LocaleProvider";

export type SupervisorStatus = {
  connection: "connected" | "disconnected";
  version: string | null;
  errorCode?: string;
};

const supervisorApi = {
  status: () => invoke<SupervisorStatus>("supervisor_status"),
  ping: () => invoke<SupervisorStatus>("ping_supervisor"),
  create: (request: MissionCommandRequest) => invoke<MissionCommandResult>("create_mission", { request }),
  launch: (request: MissionCommandRequest) => invoke<MissionCommandResult>("launch_route", { request }),
  subscribe: (request: MissionCommandRequest) => invoke<MissionCommandResult>("subscribe_mission", { request }),
  pause: (request: MissionCommandRequest) => invoke<MissionCommandResult>("request_safe_pause", { request }),
  requestForceTermination: (request: MissionCommandRequest) => invoke<MissionCommandResult>("request_force_termination", { request }),
  forceTerminate: (request: MissionCommandRequest) => invoke<MissionCommandResult>("force_terminate", { request }),
  buildRecovery: (request: MissionCommandRequest) => invoke<MissionCommandResult>("build_recovery_package", { request }),
  verifyRecovery: (request: MissionCommandRequest) => invoke<MissionCommandResult>("verify_recovery", { request }),
  resolveRecovery: (request: MissionCommandRequest) => invoke<MissionCommandResult>("resolve_recovery", { request }),
  handoff: (request: MissionCommandRequest) => invoke<MissionCommandResult>("handoff_provider", { request }),
  reviewMemory: (request: MissionCommandRequest) => invoke<MissionCommandResult>("review_memory", { request }),
  resolveApproval: (request: MissionCommandRequest) => invoke<MissionCommandResult>("resolve_approval", { request }),
  storagePreview: (request: MissionCommandRequest) => invoke<MissionCommandResult>("storage_preview", { request }),
  exportPreview: (request: MissionCommandRequest) => invoke<MissionCommandResult>("export_preview", { request }),
  diagnosticPreview: (request: MissionCommandRequest) => invoke<MissionCommandResult>("diagnostic_preview", { request }),
  archive: (request: MissionCommandRequest) => invoke<MissionCommandResult>("archive_mission", { request }),
  delete: (request: MissionCommandRequest) => invoke<MissionCommandResult>("delete_mission", { request }),
  materializeExport: (request: MissionCommandRequest) => invoke<MissionCommandResult>("materialize_export", { request }),
};

type MissionCommandRequest = {
  provider?: "codex" | "claude" | "opencode" | "zcode";
  targetProvider?: "codex" | "claude" | "opencode" | "zcode";
  missionId?: string;
  routeId?: string;
  expectedVersion?: number;
  projectRoot?: string;
  goal?: string;
  reason?: string;
  loadoutFingerprint?: string;
  resumeToken?: string;
  checkpointId?: string;
  contractVersion?: number;
  ledgerSequence?: number;
  contextPackHash?: string;
  pendingApprovalHash?: string;
  memoryId?: string;
  memoryDecision?: MemoryDecision;
  projectLimitBytes?: number;
  globalLimitBytes?: number;
  confirmationToken?: string;
  approvalId?: string;
  approvalDecision?: ApprovalResolution;
  approvalScope?: "once" | "route";
  actionDigest?: string;
  expectedRevision?: number;
  nowMs?: number;
  recoveryPackage?: Record<string, unknown>;
  recoveryDecision?: "continue" | "abandon";
  impactHash?: string;
  archivePlan?: Record<string, unknown>;
  deletePlan?: Record<string, unknown>;
};

type MissionCommandResult = {
  accepted: boolean;
  missionId: string | null;
  routeId: string | null;
  sequence: number | null;
  errorCode?: string | null;
  confirmationToken?: string | null;
  events: Array<Record<string, unknown>>;
  capability?: Record<string, unknown> | null;
  recoveryPackage?: Record<string, unknown> | null;
  capabilities?: Array<Record<string, unknown>>;
  ccSwitch?: Record<string, unknown> | null;
  data?: unknown;
  recoveryRequired?: boolean;
};

type ActiveMission = {
  draft: MissionDraft;
  missionId: string;
  routeId: string;
  lastSequence: number;
  phase: string;
  status: MissionReadModel["status"];
  currentAction: string | null;
  reason: string | null;
};

const ACTIVE_MISSION_KEY = "mission-control.active-mission.v1";

export default function App() {
  const { t } = useLocale();
  const restored = useRef(loadActiveMission()).current;
  const [status, setStatus] = useState<SupervisorStatus | null>(null);
  const [draft, setDraft] = useState<MissionDraft | null>(restored?.draft ?? null);
  const [missionId, setMissionId] = useState<string | null>(restored?.missionId ?? null);
  const [routeId, setRouteId] = useState<string | null>(restored?.routeId ?? null);
  const [recoveryPackage, setRecoveryPackage] = useState<RecoveryReviewManifest | null>(null);
  const [recoveryPackageValue, setRecoveryPackageValue] = useState<Record<string, unknown> | null>(null);
  const recoveryPackageValueRef = useRef<Record<string, unknown> | null>(null);
  const [recoveryVerified, setRecoveryVerified] = useState(false);
  const [recoveryBuilding, setRecoveryBuilding] = useState(false);
  const [commandError, setCommandError] = useState<string | null>(null);
  const [storageSnapshot, setStorageSnapshot] = useState<StorageSnapshot | null>(null);
  const [storageImpact, setStorageImpact] = useState<StorageImpact | null>(null);
  const [exportPreview, setExportPreview] = useState<ExportPreviewData | null>(null);
  const [diagnosticPreview, setDiagnosticPreview] = useState<DiagnosticPreviewData | null>(null);
  const [previewMetrics, setPreviewMetrics] = useState<PreviewMetricsData>({ participants: [], telemetryEnabled: false });
  const [mission, setMission] = useState(() => ({
    ...emptyMission(restored?.missionId ?? "local-mission"),
    lastSequence: restored?.lastSequence ?? 0,
    phase: restored?.phase ?? "Ready",
    status: restored?.status ?? "idle",
    currentAction: restored?.currentAction ?? null,
    reason: restored?.reason ?? null,
  }));
  const pendingRequest = useRef<Promise<SupervisorStatus> | null>(null);
  const disconnectPauseSent = useRef(false);
  const lastSequence = useRef(restored?.lastSequence ?? 0);

  useEffect(() => {
    let active = true;
    let observed: Promise<SupervisorStatus> | null = null;
    const update = (request: () => Promise<SupervisorStatus>) => {
      const pending = pendingRequest.current ?? request();
      pendingRequest.current = pending;
      if (observed === pending) return;
      observed = pending;
      void pending
        .then((nextStatus) => {
          if (active) setStatus(nextStatus);
        })
        .catch(() => {
          if (active) {
            setStatus({ connection: "disconnected", version: null });
          }
        })
        .finally(() => {
          if (pendingRequest.current === pending) pendingRequest.current = null;
          if (observed === pending) observed = null;
        });
    };

    update(supervisorApi.status);
    const heartbeat = window.setInterval(() => {
      update(supervisorApi.ping);
    }, 1_000);

    return () => {
      active = false;
      window.clearInterval(heartbeat);
    };
  }, []);


  useEffect(() => {
    if (status?.connection !== "disconnected" || !missionId || mission.status !== "running" || disconnectPauseSent.current) return;
    disconnectPauseSent.current = true;
    void supervisorApi.pause({ missionId, routeId: routeId ?? undefined, reason: "UI disconnected; request safe pause" })
      .then((result) => {
        const events = toMissionEvents(result.events);
        const sequence = result.sequence ?? currentSequence(events);
        lastSequence.current = sequence;
        setMission((current) => ({
          ...current,
          phase: "Paused",
          status: "paused",
          currentAction: "Safe pause requested",
          reason: "UI disconnected; request safe pause",
          lastSequence: sequence,
          events: mergeMissionEvents(current.events, events),
        }));
      })
      .catch(() => undefined);
  }, [mission.status, missionId, routeId, status?.connection]);

  useEffect(() => {
    if (status?.connection !== "connected" || !missionId) return;
    let active = true;
    let pending = false;
    const syncMission = () => {
      if (!active || pending) return;
      pending = true;
      void supervisorApi.subscribe({ missionId, expectedVersion: lastSequence.current })
        .then((result) => {
          if (!active) return;
          setMission((current) => {
            const incoming = toMissionEvents(result.events);
            const projected = projectMissionEvents(current, incoming);
            const sequence = Math.max(projected.lastSequence, result.sequence ?? 0);
            lastSequence.current = sequence;
            const next = result.recoveryRequired && projected.status !== "completed" && projected.status !== "failed"
              ? { ...projected, phase: "Recovery required", status: "paused" as const, currentAction: null, reason: projected.reason ?? "Recovery required", lastSequence: sequence }
              : { ...projected, lastSequence: sequence };
            saveActiveMission(draft && routeId ? activeMission(draft, missionId, routeId, next) : null);
            return next;
          });
        })
        .catch(() => undefined)
        .finally(() => {
          pending = false;
        });
    };
    syncMission();
    const poll = window.setInterval(syncMission, 1_000);
    return () => {
      active = false;
      window.clearInterval(poll);
    };
  }, [draft, missionId, routeId, status?.connection]);

  useEffect(() => {
    if (status?.connection !== "connected" || !missionId) {
      setStorageSnapshot(null);
      setStorageImpact(null);
      setExportPreview(null);
      setDiagnosticPreview(null);
      return;
    }
    let active = true;
    let pending = false;
    const refresh = async () => {
      if (!active || pending) return;
      pending = true;
      const [storage, impact, exported, diagnostic] = await Promise.allSettled([
        Promise.resolve().then(() => supervisorApi.storagePreview({ missionId })),
        Promise.resolve().then(() => supervisorApi.delete({ missionId })),
        Promise.resolve().then(() => supervisorApi.exportPreview({ missionId })),
        Promise.resolve().then(() => supervisorApi.diagnosticPreview({ missionId })),
      ]);
      if (active) {
        if (storage.status === "fulfilled") setStorageSnapshot(storageSnapshotFromResult(storage.value, missionId));
        if (impact.status === "fulfilled") setStorageImpact(storageImpactFromResult(impact.value));
        if (exported.status === "fulfilled") setExportPreview(exportPreviewFromResult(exported.value, missionId));
        if (diagnostic.status === "fulfilled") setDiagnosticPreview(diagnosticPreviewFromResult(diagnostic.value, missionId));
      }
      pending = false;
    };
    void refresh();
    const poll = window.setInterval(() => { void refresh(); }, 5_000);
    return () => {
      active = false;
      window.clearInterval(poll);
    };
  }, [missionId, status?.connection]);

  const createMission = async (nextDraft: MissionDraft) => {
    setCommandError(null);
    try {
      setDraft(nextDraft);
      const provider = nextDraft.agent;
      const created = await supervisorApi.create({ provider, projectRoot: nextDraft.projectRoot, goal: nextDraft.goal });
      if (!created.accepted || !created.missionId || !created.routeId) throw new Error(created.errorCode ?? "MISSION_CREATE_FAILED");
      disconnectPauseSent.current = false;
      setMissionId(created.missionId);
      setRouteId(created.routeId);
      setRecoveryPackage(null);
      setRecoveryPackageValue(null);
      recoveryPackageValueRef.current = null;
      setRecoveryVerified(false);
      lastSequence.current = created.sequence ?? 0;
      const createdMission = { ...emptyMission(created.missionId), phase: "Contract review", status: "idle" as const, currentAction: "Read-only contract created", lastSequence: created.sequence ?? 0, events: toMissionEvents(created.events) };
      saveActiveMission(activeMission(nextDraft, created.missionId, created.routeId, createdMission));
      setMission(createdMission);
      const launched = await supervisorApi.launch({
        provider,
        missionId: created.missionId,
        routeId: created.routeId,
        projectRoot: nextDraft.projectRoot,
        loadoutFingerprint: "desktop-default",
      });
      if (!launched.accepted) throw new Error(launched.errorCode ?? "ROUTE_LAUNCH_FAILED");
      const launchEvents = toMissionEvents(launched.events);
      setMission((current) => {
        const projected = projectMissionEvents(current, launchEvents);
        const launchSequence = Math.max(projected.lastSequence, launched.sequence ?? 0);
        const next = { ...projected, lastSequence: launchSequence };
        lastSequence.current = launchSequence;
        saveActiveMission(activeMission(nextDraft, created.missionId!, created.routeId!, next));
        return next;
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const discardRecovery = async () => {
    if (!missionId || !routeId) return;
    setCommandError(null);
    try {
      const packageManifest = recoveryPackage ?? await buildRecovery();
      const packageValue = recoveryPackageValueRef.current ?? recoveryPackageValue;
      if (!packageManifest || !packageValue) throw new Error("RECOVERY_PACKAGE_REQUIRED");
      const result = await supervisorApi.resolveRecovery(recoveryRequest(packageManifest, packageValue, "abandon"));
      if (!result.accepted) throw new Error(result.errorCode ?? "RECOVERY_ABANDON_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        saveActiveMission(null);
        return { ...next, lastSequence: lastSequence.current };
      });
      setDraft(null);
      setMissionId(null);
      setRouteId(null);
      setRecoveryPackage(null);
      setRecoveryPackageValue(null);
      recoveryPackageValueRef.current = null;
      setRecoveryVerified(false);
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const archiveMission = async () => {
    if (!missionId) return;
    setCommandError(null);
    try {
      const preview = await supervisorApi.archive({ missionId });
      const plan = recordPayload(preview.data);
      if (!preview.accepted || !plan || typeof plan.impact_hash !== "string") {
        throw new Error(preview.errorCode ?? "ARCHIVE_PLAN_FAILED");
      }
      if (!window.confirm(`Archive ${plan.event_count ?? 0} mission events?`)) return;
      const result = await supervisorApi.archive({ missionId, impactHash: String(plan.impact_hash), archivePlan: plan });
      if (!result.accepted) throw new Error(result.errorCode ?? "ARCHIVE_FAILED");
      setStorageSnapshot((current) => current ? { ...current, archived: true } : current);
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const deleteMission = async () => {
    if (!missionId || !storageImpact?.plan) return;
    setCommandError(null);
    try {
      const result = await supervisorApi.delete({
        missionId,
        impactHash: storageImpact.impactHash,
        deletePlan: storageImpact.plan,
      });
      if (!result.accepted) throw new Error(result.errorCode ?? "DELETE_FAILED");
      saveActiveMission(null);
      setMissionId(null);
      setRouteId(null);
      setDraft(null);
      setStorageSnapshot(null);
      setStorageImpact(null);
      setExportPreview(null);
      setDiagnosticPreview(null);
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const exportMission = async () => {
    if (!missionId) return;
    setCommandError(null);
    try {
      const result = await supervisorApi.materializeExport({ missionId });
      const data = recordPayload(result.data);
      const content = data?.content;
      if (!result.accepted || typeof content !== "string") throw new Error(result.errorCode ?? "EXPORT_FAILED");
      const blob = new Blob([content], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `mission-${missionId}-export.json`;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const reconnectRecovery = async () => {
    if (!missionId) return;
    setCommandError(null);
    try {
      const result = await supervisorApi.subscribe({ missionId, routeId: routeId ?? undefined, expectedVersion: lastSequence.current });
      if (!result.accepted) throw new Error(result.errorCode ?? "RECOVERY_RECONNECT_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const projected = projectMissionEvents(current, events);
        const sequence = Math.max(projected.lastSequence, result.sequence ?? 0);
        const next = result.recoveryRequired
          ? { ...projected, phase: "Recovery required", status: "paused" as const, currentAction: "Reconnected; recovery decision required", reason: projected.reason ?? "Recovery required", lastSequence: sequence }
          : { ...projected, lastSequence: sequence, currentAction: "Reconnected to mission" };
        lastSequence.current = sequence;
        return next;
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const pauseMission = async () => {
    if (!missionId) return;
    const result = await supervisorApi.pause({ missionId, reason: "user requested safe pause" });
    const events = toMissionEvents(result.events);
    const sequence = result.sequence ?? currentSequence(events);
    lastSequence.current = sequence;
    setMission((current) => ({ ...current, phase: "Paused", status: "paused", currentAction: "Safe pause requested", reason: "user requested safe pause", lastSequence: sequence, events: mergeMissionEvents(current.events, events) }));
  };

  const forceTerminateMission = async () => {
    if (!missionId) return;
    setCommandError(null);
    try {
      const tokenResult = await supervisorApi.requestForceTermination({ missionId, routeId: routeId ?? undefined });
      if (!tokenResult.accepted || !tokenResult.confirmationToken) {
        throw new Error(tokenResult.errorCode ?? "FORCE_TERMINATE_TOKEN_UNAVAILABLE");
      }
      const result = await supervisorApi.forceTerminate({
        missionId,
        routeId: routeId ?? undefined,
        confirmationToken: tokenResult.confirmationToken,
      });
      if (!result.accepted) throw new Error(result.errorCode ?? "FORCE_TERMINATE_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        return {
          ...next,
          phase: "Terminated",
          status: "failed",
          currentAction: null,
          reason: "Force terminated by user",
          lastSequence: lastSequence.current,
        };
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const buildRecovery = async (): Promise<RecoveryReviewManifest | null> => {
    if (!missionId || !routeId) return null;
    setRecoveryBuilding(true);
    setCommandError(null);
    try {
      const flight = toFlightViewModel(mission);
      const latest = (kind: string) => [...mission.events].reverse().find((event) => event.kind === kind)?.payload ?? {};
      const loadout = latest("loadout_snapshot");
      const context = latest("context_pack_built");
      const approval = latest("approval_requested");
      const result = await supervisorApi.buildRecovery({
        missionId,
        routeId,
        checkpointId: stringPayload(latest("checkpoint_created"), "checkpoint_id") ?? `checkpoint-${mission.lastSequence}`,
        contractVersion: numberPayload(latest("contract_updated"), "contract_version") ?? flight.contract.version,
        ledgerSequence: mission.lastSequence,
        loadoutFingerprint: stringPayload(loadout, "fingerprint") ?? "desktop-default",
        contextPackHash: stringPayload(context, "hash") ?? "desktop-context",
        pendingApprovalHash: stringPayload(approval, "pending_approval_hash"),
      });
      const manifest = recoveryManifestFromResult(result.recoveryPackage);
      if (!manifest) throw new Error(result.errorCode ?? "RECOVERY_PACKAGE_MISSING");
      setRecoveryPackage(manifest);
      const packageValue = recordPayload(result.recoveryPackage);
      recoveryPackageValueRef.current = packageValue;
      setRecoveryPackageValue(packageValue);
      setRecoveryVerified(false);
      return manifest;
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      setRecoveryBuilding(false);
    }
  };

  const reviewMemory = async (memoryId: string, memoryDecision: MemoryDecision) => {
    if (!missionId || !routeId) return;
    setCommandError(null);
    try {
      const result = await supervisorApi.reviewMemory({ missionId, routeId, memoryId, memoryDecision, expectedVersion: mission.lastSequence });
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        return { ...next, lastSequence: lastSequence.current };
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const resolveApproval = async (approvalId: string, approvalDecision: ApprovalResolution) => {
    if (!missionId || !routeId) return;
    setCommandError(null);
    try {
      const source = [...mission.events].reverse().find((event) => {
        if (!['approval_requested', 'approval_resolved', 'approval_revoked'].includes(event.kind)) return false;
        const approval = recordPayload(event.payload.approval);
        return stringPayload(event.payload, "approval_id") === approvalId || stringPayload(approval ?? {}, "id") === approvalId;
      });
      const payload = source?.payload ?? {};
      const approval = recordPayload(payload.approval) ?? payload;
      const subject = recordPayload(approval.subject) ?? recordPayload(payload.subject) ?? {};
      const actionDigest = stringPayload(approval, "action_digest") ?? stringPayload(subject, "action_digest");
      const contractVersion = numberPayload(approval, "contract_version") ?? numberPayload(subject, "contract_version");
      const loadoutFingerprint = stringPayload(approval, "loadout_fingerprint") ?? stringPayload(subject, "loadout_fingerprint");
      const expectedRevision = numberPayload(approval, "revision") ?? numberPayload(payload, "expected_revision");
      if (!actionDigest || contractVersion == null || !loadoutFingerprint || expectedRevision == null) {
        throw new Error("APPROVAL_BINDING_INCOMPLETE");
      }
      const result = await supervisorApi.resolveApproval({
        missionId,
        routeId,
        approvalId,
        approvalDecision,
        approvalScope: approvalDecision === "approve-route" ? "route" : "once",
        actionDigest,
        expectedRevision,
        contractVersion,
        loadoutFingerprint,
        nowMs: Date.now(),
      });
      if (!result.accepted) throw new Error(result.errorCode ?? "APPROVAL_RESOLUTION_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        return { ...next, lastSequence: lastSequence.current };
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const handoffProvider = async (targetProvider: "codex" | "claude") => {
    if (!missionId || !routeId) return;
    setCommandError(null);
    try {
      const manifest = recoveryPackage ?? await buildRecovery();
      if (!manifest) return;
      const result = await supervisorApi.handoff({
        missionId,
        routeId,
        targetProvider,
        contextPackHash: manifest.contextPackHash,
        pendingApprovalHash: manifest.pendingApprovalHash ?? undefined,
      });
      if (!result.accepted) throw new Error(result.errorCode ?? "HANDOFF_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        return { ...next, lastSequence: lastSequence.current };
      });
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const verifyRecovery = async () => {
    if (!recoveryPackage || !missionId || !routeId) return;
    const packageValue = recoveryPackageValueRef.current ?? recoveryPackageValue;
    if (!packageValue) {
      setCommandError("RECOVERY_PACKAGE_REQUIRED");
      return;
    }
    setCommandError(null);
    try {
      const result = await supervisorApi.verifyRecovery(recoveryRequest(recoveryPackage, packageValue));
      if (!result.accepted || result.data && recordPayload(result.data)?.verified !== true) {
        throw new Error(result.errorCode ?? "RECOVERY_VERIFY_FAILED");
      }
      setRecoveryVerified(true);
    } catch (error) {
      setRecoveryVerified(false);
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  const resumeRecovery = async () => {
    if (!recoveryVerified || !recoveryPackage || !missionId || !routeId) return;
    const packageValue = recoveryPackageValueRef.current ?? recoveryPackageValue;
    if (!packageValue) {
      setCommandError("RECOVERY_PACKAGE_REQUIRED");
      return;
    }
    setCommandError(null);
    try {
      const result = await supervisorApi.resolveRecovery(recoveryRequest(recoveryPackage, packageValue, "continue"));
      if (!result.accepted) throw new Error(result.errorCode ?? "RECOVERY_RESUME_FAILED");
      const events = toMissionEvents(result.events);
      setMission((current) => {
        const next = projectMissionEvents(current, events);
        lastSequence.current = Math.max(next.lastSequence, result.sequence ?? 0);
        saveActiveMission(draft ? activeMission(draft, missionId, routeId, { ...next, lastSequence: lastSequence.current }) : null);
        return { ...next, lastSequence: lastSequence.current };
      });
      setRecoveryVerified(false);
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };

  let message = t("app.connecting");
  if (status?.connection === "connected") {
    message = status.version
      ? t("app.connectedVersion", { version: status.version })
      : t("app.connected");
  } else if (status?.connection === "disconnected") {
    message = t("app.disconnected");
  }
  const state = status?.connection ?? "connecting";
  const hasMission = Boolean(draft && missionId);
  const displayOverride = state === "connected"
    ? hasMission ? undefined : <NewMission embedded onCreate={(nextDraft) => { void createMission(nextDraft); }} />
    : <ConnectionDisplay message={message} state={state} />;

  return <>
    <div className="supervisor-status sr-only" role="status" aria-live="polite">{message}</div>
    <div className="mission-workspace"><BasicMissionFlight
      mission={mission}
      events={mission.events}
      connectionState={state}
      displayOverride={displayOverride}
       onPause={state === "connected" && missionId ? () => { void pauseMission(); } : undefined}
       onForceTerminate={state === "connected" && missionId && mission.status !== "completed" && mission.status !== "failed" ? () => { void forceTerminateMission(); } : undefined}
      onReconnect={reconnectRecovery}
      onDiscard={discardRecovery}
      recoveryPackage={recoveryPackage}
      recoveryVerified={recoveryVerified}
      recoveryBuilding={recoveryBuilding}
      onBuildRecovery={() => { void buildRecovery(); }}
      onVerifyRecovery={verifyRecovery}
      onResumeRecovery={resumeRecovery}
      onMemoryDecision={(id, decision) => { void reviewMemory(id, decision); }}
      onResolveApproval={(id, decision) => { void resolveApproval(id, decision); }}
      onHandoff={(provider) => { void handoffProvider(provider); }}
      storageSnapshot={storageSnapshot ?? undefined}
      storageImpact={storageImpact}
      onArchiveMission={() => { void archiveMission(); }}
      onDeleteMission={() => { void deleteMission(); }}
      onExportMission={() => { void exportMission(); }}
      exportPreview={exportPreview}
      diagnosticPreview={diagnosticPreview}
      previewMetrics={previewMetrics}
      onPreviewTelemetryChange={(enabled) => setPreviewMetrics((current) => ({ ...current, telemetryEnabled: enabled }))}
      onExportPreviewMetrics={() => exportPreviewMetrics(previewMetrics)}
    /></div>
    {commandError && <div className="supervisor-command-error" role="alert">{commandError}</div>}
  </>;
}

function exportPreviewMetrics(data: PreviewMetricsData): void {
  const blob = new Blob([JSON.stringify({ schema: "mission-control-codex-preview-export-v1", exportedAtUtc: new Date().toISOString(), telemetryEnabled: data.telemetryEnabled, sourceIncluded: false, secretIncluded: false }, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "codex-preview-redacted-receipt.json";
  anchor.click();
  URL.revokeObjectURL(url);
}

function ConnectionDisplay({ message, state }: { message: string; state: "connecting" | "disconnected" }) {
  const { t } = useLocale();
  return <section className="cockpit-standby" data-connection={state} aria-label={message}>
    <div className="cockpit-standby__scope" aria-hidden="true"><i /><i /><span /></div>
    <span className="panel-kicker">{t("beam.control")}</span>
    <h1>{message}</h1>
    <p>{state === "connecting" ? t("new.uplinkReady") : t("systems.recovery")}</p>
  </section>;
}

function stringPayload(payload: Record<string, unknown>, key: string): string | undefined {
  const value = payload[key] ?? payload[key.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())];
  return typeof value === "string" && value.trim() ? value : undefined;
}

function numberPayload(payload: Record<string, unknown>, key: string): number | undefined {
  const value = payload[key] ?? payload[key.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function recordPayload(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function stringListPayload(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string" && item.trim().length > 0) : [];
}

function storageSnapshotFromResult(result: MissionCommandResult, missionId: string): StorageSnapshot | null {
  const data = recordPayload(result.data);
  const usage = recordPayload(data?.project_usage ?? data?.projectUsage);
  if (!data || !usage) return null;
  const usedBytes = numberPayload(usage, "total_bytes");
  const eventCount = numberPayload(usage, "event_count");
  const budget = recordPayload(data.budget);
  const budgetBytes = budget ? numberPayload(budget, "project_limit_bytes") ?? null : null;
  if (usedBytes == null || eventCount == null) return null;
  return { missionId, usedBytes, eventCount, archived: false, budgetBytes };
}

function storageImpactFromResult(result: MissionCommandResult): StorageImpact | null {
  const data = recordPayload(result.data);
  if (!data) return null;
  const impactHash = stringPayload(data, "impact_hash");
  const eventCount = numberPayload(data, "event_count");
  const bytes = numberPayload(data, "bytes");
  const blobs = Array.isArray(data.blob_refs) ? data.blob_refs : [];
  if (!impactHash || eventCount == null || bytes == null) return null;
  return {
    impactHash,
    projectedBytes: bytes,
    affectedEvents: eventCount,
    affectedBlobs: blobs.length,
    automaticDeletion: false,
    blobs: blobs.flatMap((value) => {
      const blob = recordPayload(value);
      const hash = blob ? stringPayload(blob, "hash") : undefined;
      const size = blob ? numberPayload(blob, "size") : undefined;
      if (!hash || size == null) return [];
      return [{ hash, size, willRemove: blob?.will_remove === true || blob?.willRemove === true }];
    }),
    plan: data,
  };
}

function exportPreviewFromResult(result: MissionCommandResult, missionId: string): ExportPreviewData | null {
  const data = recordPayload(result.data);
  if (!data) return null;
  const eventCount = numberPayload(data, "event_count");
  const sizeBytes = numberPayload(data, "size_bytes");
  const contentHash = stringPayload(data, "content_hash");
  if (eventCount == null || sizeBytes == null || !contentHash) return null;
  return {
    missionId: stringPayload(data, "mission_id") ?? missionId,
    eventCount,
    sizeBytes,
    contentHash,
    categories: stringListPayload(data.categories),
    containsRawProviderPayload: data.contains_raw_provider_payload === true || data.containsRawProviderPayload === true,
  };
}

function diagnosticPreviewFromResult(result: MissionCommandResult, missionId: string): DiagnosticPreviewData | null {
  const data = recordPayload(result.data);
  if (!data) return null;
  const eventCount = numberPayload(data, "event_count");
  const exportHash = stringPayload(data, "export_hash");
  if (eventCount == null || !exportHash) return null;
  const ledger = recordPayload(data.ledger);
  const lastCommittedSequence = ledger ? numberPayload(ledger, "last_committed_sequence") : undefined;
  const recoveryRequired = ledger?.recovery_required === true || ledger?.recoveryRequired === true;
  return {
    missionId: stringPayload(data, "mission_id") ?? missionId,
    eventCount,
    exportHash,
    redactionCategories: stringListPayload(data.redaction_categories ?? data.redactionCategories),
    telemetryEnabled: data.telemetry_enabled === true || data.telemetryEnabled === true,
    includesSource: data.includes_source === true || data.includesSource === true,
    includesProviderPayload: data.includes_provider_payload === true || data.includesProviderPayload === true,
    ledger: lastCommittedSequence == null ? undefined : { lastCommittedSequence, recoveryRequired },
  };
}

function recoveryManifestFromResult(value: Record<string, unknown> | null | undefined): RecoveryReviewManifest | null {
  const manifest = value && typeof value.manifest === "object" && value.manifest !== null ? value.manifest as Record<string, unknown> : null;
  if (!manifest) return null;
  const stringField = (snake: string) => stringPayload(manifest, snake);
  const numberField = (snake: string) => numberPayload(manifest, snake);
  const missionId = stringField("mission_id");
  const routeId = stringField("route_id");
  const checkpointId = stringField("checkpoint_id");
  const loadoutFingerprint = stringField("loadout_fingerprint");
  const contextPackHash = stringField("context_pack_hash");
  const entryHash = stringField("entry_hash");
  const schemaVersion = numberField("schema_version");
  const contractVersion = numberField("contract_version");
  const ledgerSequence = numberField("ledger_sequence");
  if (!missionId || !routeId || !checkpointId || !loadoutFingerprint || !contextPackHash || !entryHash || schemaVersion == null || contractVersion == null || ledgerSequence == null) return null;
  const pendingApprovalHash = stringField("pending_approval_hash") ?? null;
  return { missionId, routeId, schemaVersion, contractVersion, checkpointId, ledgerSequence, loadoutFingerprint, contextPackHash, pendingApprovalHash, entryHash };
}

function recoveryRequest(
  manifest: RecoveryReviewManifest,
  packageValue: Record<string, unknown>,
  decision?: "continue" | "abandon",
): MissionCommandRequest {
  return {
    missionId: manifest.missionId,
    routeId: manifest.routeId,
    contractVersion: manifest.contractVersion,
    ledgerSequence: manifest.ledgerSequence,
    loadoutFingerprint: manifest.loadoutFingerprint,
    contextPackHash: manifest.contextPackHash,
    pendingApprovalHash: manifest.pendingApprovalHash ?? undefined,
    recoveryPackage: packageValue,
    recoveryDecision: decision,
  };
}

function toMissionEvents(events: Array<Record<string, unknown>>): MissionEvent[] {
  return events.map((event) => ({
    missionId: String(event.mission_id ?? ""),
    sequence: Number(event.sequence ?? 0),
    kind: String(event.kind ?? "unknown"),
    payload: (event.payload as Record<string, unknown> | undefined) ?? {},
    source: (event.source as MissionEvent["source"]) ?? "supervisor",
  }));
}

function mergeMissionEvents(current: MissionEvent[], incoming: MissionEvent[]): MissionEvent[] {
  const bySequence = new Map(current.map((event) => [event.sequence, event]));
  for (const event of incoming) bySequence.set(event.sequence, event);
  return [...bySequence.values()].sort((left, right) => left.sequence - right.sequence);
}

function currentSequence(events: MissionEvent[]): number {
  return events.reduce((sequence, event) => Math.max(sequence, event.sequence), 0);
}

function projectMissionEvents(current: MissionReadModel, incoming: MissionEvent[]): MissionReadModel {
  let state = { [current.missionId]: current };
  for (const event of incoming.sort((left, right) => left.sequence - right.sequence)) {
    state = reduceMission(state, { type: "event", event });
  }
  return state[current.missionId] ?? current;
}

function loadActiveMission(): ActiveMission | null {
  try {
    const raw = window.localStorage.getItem(ACTIVE_MISSION_KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<ActiveMission>;
    if (!value.missionId || !value.routeId || !value.draft || typeof value.lastSequence !== "number") return null;
    const hasProjection = typeof value.phase === "string"
      && ["idle", "running", "paused", "completed", "failed"].includes(value.status ?? "")
      && (typeof value.currentAction === "string" || value.currentAction === null)
      && (typeof value.reason === "string" || value.reason === null);
    if (hasProjection) return value as ActiveMission;
    return { draft: value.draft, missionId: value.missionId, routeId: value.routeId, lastSequence: 0, phase: "Ready", status: "idle", currentAction: null, reason: null };
  } catch {
    return null;
  }
}

function saveActiveMission(active: ActiveMission | null): void {
  if (active) window.localStorage.setItem(ACTIVE_MISSION_KEY, JSON.stringify(active));
  else window.localStorage.removeItem(ACTIVE_MISSION_KEY);
}

function activeMission(draft: MissionDraft, missionId: string, routeId: string, mission: MissionReadModel): ActiveMission {
  const { lastSequence, phase, status, currentAction, reason } = mission;
  return { draft, missionId, routeId, lastSequence, phase, status, currentAction, reason };
}
