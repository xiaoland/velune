// Pi v1.0.2 SDK session-list projection. CoreRuntime owns process isolation;
// Pi's SessionManager remains authoritative for session metadata.

const args = process.argv.slice(2);
const value = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const agentDir = value("--agent-dir");
if (agentDir) process.env.PI_CODING_AGENT_DIR = agentDir;
// Set the explicit instance root before SDK initialization. The Rust helper
// transport clears ambient configuration so history cannot select another HOME.
const { SessionManager } = await import("@earendil-works/pi-coding-agent");
const cwd = value("--cwd") ?? process.cwd();
const sessionDir = value("--session-dir");
const inspectPath = value("--inspect-session");
if (inspectPath) {
  const manager = SessionManager.open(inspectPath);
  const context = manager.buildSessionContext();
  const virtualEntry = [...manager.getBranch()]
    .reverse()
    .find((entry) => entry.type === "custom" && entry.customType === "pi.virtual-model-state");
  process.stdout.write(JSON.stringify({
    name: manager.getSessionName() ?? null,
    cwd: manager.getCwd(),
    messages: context.messages,
    model: context.model,
    thinkingLevel: context.thinkingLevel,
    virtualState: virtualEntry?.data ?? null,
  }));
} else {
const sessions = args.includes("--all")
  ? await SessionManager.listAll(sessionDir)
  : await SessionManager.list(cwd, sessionDir);
process.stdout.write(
  JSON.stringify({
    contract_version: 1,
    pi_sdk: "1.0.2",
    sessions: sessions.map((session) => ({
      path: session.path,
      id: session.id,
      cwd: session.cwd,
      name: session.name ?? null,
      created: session.created.toISOString(),
      modifiedUnixMs: session.modified.getTime(),
      messageCount: session.messageCount,
      firstMessage: session.firstMessage,
    })),
  }),
);

}
