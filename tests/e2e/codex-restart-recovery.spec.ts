// Contract fixture for the Playwright harness used by the desktop preview.
// The important invariant is that reconnecting subscribes after the last
// sequence and presents recovery choices without issuing a resume command.
export const restartRecoveryContract = {
  afterSequence: 12,
  choices: ["reconnect", "restart_agent", "resume_checkpoint", "discard"],
  automaticallyResumes: false,
};
