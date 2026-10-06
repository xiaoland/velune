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
const mutationPath = value("--rename-session") ?? value("--delete-session");
if (mutationPath) {
  const { realpath, lstat, unlink } = await import("node:fs/promises");
  const { resolve, relative, isAbsolute } = await import("node:path");
  const root = await realpath(sessionDir ?? resolve(agentDir, "sessions"));
  const sessions = await SessionManager.listAll(sessionDir);
  if (!sessions.some(session => session.path === mutationPath)) throw new Error("Session not in configured source");
  const actual = await realpath(mutationPath);
  const within = relative(root, actual);
  const stat = await lstat(mutationPath);
  if (stat.isSymbolicLink() || stat.nlink !== 1 || !within || within.startsWith("../") || isAbsolute(within)) throw new Error("Session source escaped configured root");
  if (args.includes("--rename-session")) {
    let input = "";
    for await (const chunk of process.stdin) input += chunk;
    const {title} = JSON.parse(input);
    if (typeof title !== "string" || !title.trim()) throw new Error("Nonempty title required");
    const manager = SessionManager.open(mutationPath);
    manager.appendSessionInfo(title.trim());
    if (SessionManager.open(mutationPath).getSessionName() !== title.trim()) throw new Error("Session rename was not persisted");
  } else {
    // Pi's own selector uses unlink when trash is unavailable. Velune's native
    // confirmation explicitly describes permanent deletion of this source file.
    await unlink(mutationPath);
  }
  process.stdout.write(JSON.stringify({ok:true}));
} else
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
