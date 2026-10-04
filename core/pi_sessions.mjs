// Pi v1.0.2 SDK session-list projection. The Host owns process isolation;
// Pi's SessionManager remains authoritative for session metadata.
import { SessionManager } from "@earendil-works/pi-coding-agent";

const args = process.argv.slice(2);
const value = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const cwd = value("--cwd") ?? process.cwd();
const sessionDir = value("--session-dir");
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
      modified: session.modified.toISOString(),
      messageCount: session.messageCount,
      firstMessage: session.firstMessage,
    })),
  }),
);
