// Pi v1.0.2 SDK session-list projection. CoreRuntime owns process isolation;
// Pi's SessionManager remains authoritative for session metadata.

import { resolvePiSdk } from "./pi_sdk.mjs";

const args = process.argv.slice(2);
const value = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const agentDir = value("--agent-dir");
if (agentDir) process.env.PI_CODING_AGENT_DIR = agentDir;
// Set the explicit instance root before SDK initialization. The Rust helper
// transport clears ambient configuration so history cannot select another HOME.
// Module loading dominates small reads and unchanged listings. Import only the
// version-pinned session module, and only when Pi must read or modify a session.
const installation = resolvePiSdk({binary:value("--cli"),runtimeTypeId:value("--runtime-type")});
const sessionManager = () => installation.importModule("dist/core/session-manager.js");
const sessionDir = value("--session-dir");
const inspectPath = value("--inspect-session");
const mutationPath = value("--rename-session") ?? value("--delete-session");
if (mutationPath) {
  const { SessionManager } = await sessionManager();
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
  const { SessionManager } = await sessionManager();
  const manager = SessionManager.open(inspectPath);
  const context = manager.buildSessionContext();
  const virtualEntry = [...manager.getBranch()]
    .reverse()
    .find((entry) => entry.type === "custom" && entry.customType === "pi.virtual-model-state");
  process.stdout.write(JSON.stringify({
    name: manager.getSessionName() ?? null,
    cwd: manager.getCwd(),
    messages: manager.buildSessionProjection().entries.flatMap(entry =>
      entry.messages.map((message, ordinal) => ({...message, id:`pi:${manager.getSessionId()}:${entry.sourceEntry.id}:${ordinal}`}))),
    model: context.model,
    thinkingLevel: context.thinkingLevel,
    virtualState: virtualEntry?.data ?? null,
  }));
} else {
  // Discovery mirrors SessionManager.listAll. Every file is reported with
  // its identity and, from its first line, the working directory and start
  // time it records, so the list can group and count without reading
  // sessions. Only the files the caller names are parsed, by Pi's own
  // metadata reader through a private directory of links, so Pi stays
  // authoritative for every listed field.
  const { readdir, stat, mkdtemp, symlink, rm, open } = await import("node:fs/promises");
  const { join, resolve } = await import("node:path");
  const { tmpdir } = await import("node:os");
  let input = "";
  for await (const chunk of process.stdin) input += chunk;
  const want = new Set(JSON.parse(input).want);
  const sessionsRoot = resolve(agentDir, "sessions");
  const directories = sessionDir
    ? [sessionDir]
    : await readdir(sessionsRoot, { withFileTypes: true })
        .then((entries) => entries.filter((entry) => entry.isDirectory() || entry.isSymbolicLink()).map((entry) => join(sessionsRoot, entry.name)))
        .catch(() => []);
  const paths = (await Promise.all(directories.map((directory) => readdir(directory)
    .then((names) => names.filter((name) => name.endsWith(".jsonl")).map((name) => join(directory, name)))
    .catch(() => [])))).flat();
  const header = async (path) => {
    const file = await open(path, "r");
    try {
      const buffer = Buffer.alloc(65536);
      const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
      const line = buffer.subarray(0, bytesRead).toString("utf8").split("\n")[0];
      const value = JSON.parse(line);
      return value?.type === "session" ? value : null;
    } catch {
      return null;
    } finally {
      await file.close();
    }
  };
  // Nanoseconds as a decimal string survive the JSON round trip exactly.
  const files = (await Promise.all(paths.map(async (path) => {
    try {
      const stats = await stat(path, { bigint: true });
      const start = await header(path);
      const created = typeof start?.timestamp === "string" ? Date.parse(start.timestamp) : NaN;
      return { path, size: String(stats.size), mtimeNs: String(stats.mtimeNs),
        modifiedAtUnixMs: Number(stats.mtimeMs),
        cwd: typeof start?.cwd === "string" && start.cwd ? start.cwd : null,
        createdAtUnixMs: Number.isFinite(created) ? created : null };
    } catch {
      return null;
    }
  }))).filter((file) => file !== null);
  const wanted = files.filter((file) => want.has(file.path));
  let sessions = [];
  if (wanted.length) {
    const { SessionManager } = await sessionManager();
    const links = await mkdtemp(join(tmpdir(), "velune-pi-list-"));
    try {
      await Promise.all(wanted.map((file, index) => symlink(file.path, join(links, `${index}.jsonl`))));
      sessions = (await SessionManager.listAll(links)).map((session) => {
        const index = Number(session.path.slice(links.length + 1, -".jsonl".length));
        return {
          path: wanted[index].path,
          id: session.id,
          cwd: session.cwd,
          name: session.name ?? null,
          createdUnixMs: session.created.getTime(),
          modifiedUnixMs: session.modified.getTime(),
          messageCount: session.messageCount,
          firstMessage: session.firstMessage,
        };
      });
    } finally {
      await rm(links, { recursive: true, force: true });
    }
  }
  process.stdout.write(JSON.stringify({ contractVersion: 3, files, sessions }));
}
