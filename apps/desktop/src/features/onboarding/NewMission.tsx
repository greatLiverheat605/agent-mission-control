import { useState, type FormEvent } from "react";

export type MissionDraft = { projectRoot: string; goal: string; agent: "codex" | "claude" };

export function NewMission({ onCreate }: { onCreate: (draft: MissionDraft) => void }) {
  const [projectRoot, setProjectRoot] = useState("");
  const [goal, setGoal] = useState("");
  const [agent, setAgent] = useState<MissionDraft["agent"]>("codex");
  const submit = (event: FormEvent) => { event.preventDefault(); if (projectRoot.trim() && goal.trim()) onCreate({ projectRoot: projectRoot.trim(), goal: goal.trim(), agent }); };
  return <form className="mission-panel" onSubmit={submit} aria-labelledby="new-mission-title">
    <div className="eyebrow">Mission setup</div>
    <h1 id="new-mission-title">Start a read-only mission</h1>
    <p className="muted">Review the contract before any agent session starts.</p>
    <label>Project folder<input aria-label="Project folder" value={projectRoot} onChange={(e) => setProjectRoot(e.target.value)} placeholder="C:\\workspace\\project" /></label>
    <label>Goal<textarea aria-label="Mission goal" value={goal} onChange={(e) => setGoal(e.target.value)} placeholder="Inspect the repository and report risks" rows={3} /></label>
    <label>Agent<select aria-label="Agent" value={agent} onChange={(e) => setAgent(e.target.value as MissionDraft["agent"])}><option value="codex">Codex</option><option value="claude">Claude</option></select></label>
    <button type="submit">Review contract</button>
  </form>;
}
