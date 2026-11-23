// Structured bridge to pi-background-tasks' public Host EventBus API v1.
// The RPC stdin stream stays owned by Pi's SDK; requests cross it only as
// ordinary Velune extension commands and versioned setStatus frames.
import { randomUUID } from "node:crypto";
import {indexSubagentRuns, subagentSessions, subagentTitle, readSubagentMetadata} from "./pi_subagents.mjs";
import { readdir, open, lstat } from "node:fs/promises";
import { dirname, basename, join, isAbsolute } from "node:path";

const REQUEST = "pi-background-tasks:host:request:v1";
const RESPONSE = "pi-background-tasks:host:response:v1";
const CHANGED = "pi-background-tasks:host:changed:v1";
const CLIENT_ID = "velune-pi-runtime-v1";
const STATUS_KEY = "velune.background-work-v1";
const FRAME_VERSION = 1;
const REQUEST_TIMEOUT_MS = 1500;
const canonicalRunId = value => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(value);
const canonicalRunSlot = value => /^run-(0|[1-9][0-9]{0,9})$/u.test(value) && Number(value.slice(4)) <= 0xffffffff;
const boundedOutput = value => {
  const bytes = Buffer.from(typeof value === "string" ? value : "");
  let end = Math.min(bytes.length, 65536);
  while (end > 0 && end < bytes.length && (bytes[end] & 0xc0) === 0x80) end -= 1;
  return {text:bytes.subarray(0, end).toString("utf8"),truncated:end < bytes.length};
};

const compactError = (error) => {
  const text = error instanceof Error ? error.message : String(error);
  return text.replace(/\s+/gu, " ").trim().slice(0, 512) || "unknown host API error";
};

export default function installPiBackgroundWork(pi) {
  const pending = new Map();
  let latestContext;
  let disposed = false;
  let host;
  let hostObserved = false;
  let hostOperations = new Set();
  let lease;
  let sequence = 0;
  let refreshPromise;
  let refreshAgain = false;
  let refreshFailures = 0;
  let retryTimer;
  let children = [];
  let childrenUnknown = false;
  let subagentSupported;
  let subagentObserved = false;
  let childReadSequence = 0;
  let metadataKey;
  let archivedRuns = new Map();
  let subAgents = [];
  let childParent;
  const useChildParent = parent => {
    if (childParent === parent) return;
    childParent = parent;
    childReadSequence += 1;
    metadataKey = undefined;
    archivedRuns = new Map();
    subAgents = [];
    children = [];
    childrenUnknown = false;
    lastFrame = {version:FRAME_VERSION,kind:"uninitialized",tasks:[]};
  };
  // Public registries distinguish a silent installed extension from a missing
  // responder. Disabled tools are still returned by getAllTools.
  const registeredSubagents = () => {
    try {
      const tools = pi.getAllTools?.();
      const commands = pi.getCommands?.();
      if (tools?.some(tool => ["subagent", "subagents_enable"].includes(tool.name))
          || commands?.some(command => command.name === "subagents-inspect-rpc")) return true;
      return Array.isArray(tools) && Array.isArray(commands) ? false : undefined;
    } catch { return undefined; }
  };
  const subagentRpc = (method, params = {}) => new Promise((resolve, reject) => {
    const requestId = randomUUID();
    const event = `subagents:rpc:v1:reply:${requestId}`;
    let remove = () => {};
    const timer = setTimeout(() => { remove(); reject(Object.assign(new Error("Pi subagent RPC timed out"), {code:"subagent_timeout"})); }, REQUEST_TIMEOUT_MS);
    remove = pi.events.on(event, reply => {
      subagentObserved = true;
      clearTimeout(timer); remove();
      if (reply?.version !== 1 || reply.requestId !== requestId || reply.method !== method) {
        reject(new Error("Invalid Pi subagent RPC reply"));
        return;
      }
      if (reply.success) resolve(reply.data); else reject(new Error(reply.error?.message ?? "Pi subagent RPC rejected"));
    });
    pi.events.emit("subagents:rpc:v1:request", {version:1, requestId, method, params});
  });
  const readChildren = async ctx => {
    if (subagentSupported === false) return;
    const parent = ctx.sessionManager.getSessionFile();
    if (latestContext && parent !== latestContext.sessionManager.getSessionFile()) return;
    useChildParent(parent);
    const request = ++childReadSequence;
    try {
      const entries = ctx.sessionManager.getEntries?.() ?? [];
      const key = JSON.stringify([parent, entries.length, entries.at(-1)?.id]);
      if (parent && key !== metadataKey) {
        const metadata = await readSubagentMetadata(parent, entries);
        if (request !== childReadSequence || disposed) return;
        archivedRuns = indexSubagentRuns(entries);
        const names = new Map(subAgents.map(child => [child.sessionFile, child]));
        for (const child of metadata) names.set(child.sessionFile, child);
        subAgents = [...names.values()];
        metadataKey = key;
      }
      if (subagentSupported === undefined) {
        const caps = await subagentRpc("ping");
        if (caps?.capabilities?.asyncStatusSnapshot?.version !== 1 || !Array.isArray(caps.methods) || !caps.methods.includes("stop")) {
          throw new Error("Unknown Pi subagent capabilities");
        }
        subagentSupported = true;
      }
      const response = await subagentRpc("status");
      const snapshot = response.asyncSnapshot;
      if (snapshot?.version !== 1 || !Array.isArray(snapshot.runs)) throw new Error("Unknown Pi subagent snapshot");
      if (snapshot.omitted?.runs > 0 || snapshot.omitted?.children > 0 || snapshot.omitted?.byteLimitExceeded) throw new Error("Pi subagent snapshot is incomplete");
      const next = [], activeRuns = new Set();
      let unmapped = false;
      for (const run of snapshot.runs) {
        if (!["running", "queued"].includes(run.state)) continue;
        const states = (run.children ?? []).map(child => child.state);
        // A run can still monitor/report after all of its child executions end.
        if (states.length && states.every(state => ["complete", "completed", "failed", "partial", "paused", "stopped", "rejected"].includes(state))) continue;
        activeRuns.add(run.id);
        if (!parent || !/^[a-zA-Z0-9_-]+$/.test(run.id)) { unmapped = true; continue; }
        const sessions = await subagentSessions(parent, run.id, archivedRuns);
        if (!sessions.length) { unmapped = true; continue; }
        // Mixed workflow steps cannot be matched from opaque snapshot node IDs.
        // Preserve uncertainty rather than marking terminal siblings as running.
        if (states.some(state => ["complete", "completed", "failed", "partial", "paused", "stopped", "rejected"].includes(state))) {
          unmapped = true;
          continue;
        }
        const single = run.kind === "subagent" && run.children?.length === 1 && sessions.length === 1;
        for (const sessionFile of sessions) {
          const title = subagentTitle(archivedRuns.get(run.id), archivedRuns, run.label ?? "子 Agent", sessionFile);
          next.push({sessionFile, title, runId:run.id, canStop:single, state:"running"});
        }
      }
      if (request !== childReadSequence || disposed || parent !== ctx.sessionManager.getSessionFile()) return;
      // An older slow snapshot must not resurrect work after a newer completion.
      children = unmapped
        ? [...next, ...children.filter(child => activeRuns.has(child.runId) && !next.some(known => known.sessionFile === child.sessionFile))]
        : next;
      childrenUnknown = unmapped;
      const claims = new Map();
      for (const child of children) {
        if (!claims.has(child.sessionFile)) claims.set(child.sessionFile, new Set());
        claims.get(child.sessionFile).add(child.runId);
      }
      children = children.map(child => ({...child, canStop:child.canStop && !unmapped && claims.get(child.sessionFile).size === 1}));
      const names = new Map(subAgents.map(child => [child.sessionFile, child]));
      for (const child of children) if (!names.has(child.sessionFile)) names.set(child.sessionFile, {sessionFile:child.sessionFile,title:child.title,failed:false});
      subAgents = [...names.values()];
    } catch (error) {
      if (request !== childReadSequence || disposed) return;
      if (subagentSupported === undefined && !subagentObserved
          && error?.code === "subagent_timeout" && registeredSubagents() === false) {
        subagentSupported = false;
      } else {
        childrenUnknown = true;
        children = children.map(child => ({...child, canStop:false}));
      }
    }
  };
  let lastFrame = { version: FRAME_VERSION, kind: "uninitialized", tasks: [] };
  let removeResponseListener = () => {};
  let removeChangedListener = () => {};
  let removeSessionStartListener = () => {};
  let removeShutdownListener = () => {};

  const publish = (ctx, frame) => {
    // Child refreshes can finish before the first host observation.
    if (disposed || frame.kind === "uninitialized") return;
    if (ctx && latestContext && ctx.sessionManager.getSessionFile() !== latestContext.sessionManager.getSessionFile()) return;
    if (ctx) latestContext = ctx;
    if (!latestContext) return;
    const parentSessionFile = latestContext.sessionManager.getSessionFile();
    useChildParent(parentSessionFile);
    // Delivery is a fact of the session archive, not the current LLM context.
    // Compaction may remove an already-delivered notification from get_messages.
    const persistedNotificationIds = [...new Set((latestContext.sessionManager.getEntries?.() ?? [])
      .filter(entry => entry.type === "custom_message" && entry.customType === "background-task-notification")
      .map(entry => entry.details?.id)
      .filter(id => typeof id === "string" && id.length > 0))];
    frame = {...frame, parentSessionFile, children, subAgents, childrenUnknown, persistedNotificationIds};
    // Results are one-shot deliveries, not part of the periodically
    // republished metadata snapshot (which can otherwise repeat 64 KiB logs).
    const {taskOutput, taskOperation, ...metadata} = frame;
    lastFrame = metadata;
    latestContext.ui.setStatus(STATUS_KEY, JSON.stringify(frame));
  };

  const request = (operation, payload) => new Promise((resolve, reject) => {
    if (disposed) {
      reject(new Error("Pi background host activation is closed"));
      return;
    }
    const requestId = randomUUID();
    const timer = setTimeout(() => {
      pending.delete(requestId);
      reject(Object.assign(new Error(`Pi background host ${operation} request timed out`), {code:"background_host_timeout"}));
    }, REQUEST_TIMEOUT_MS);
    pending.set(requestId, { resolve, reject, timer, operation });
    try {
      pi.events.emit(REQUEST, {
        schema_version: "pi-background-tasks.host-request.v1",
        request_id: requestId,
        operation,
        payload,
      });
    } catch (error) {
      clearTimeout(timer);
      pending.delete(requestId);
      reject(error);
    }
  });

  removeResponseListener = pi.events.on(RESPONSE, (frame) => {
    if (frame?.schema_version !== "pi-background-tasks.host-response.v1") return;
    const item = pending.get(frame.request_id);
    if (!item) return;
    pending.delete(frame.request_id);
    hostObserved = true;
    clearTimeout(item.timer);
    if (frame.operation !== item.operation) {
      item.reject(new Error("Pi background host response operation mismatch"));
    } else if (frame.ok === true) {
      item.resolve(frame.result);
    } else {
      item.reject(new Error(`Pi background host ${frame.error?.code ?? "error"}: ${frame.error?.message ?? "request failed"}`));
    }
  });

  const leasedRequest = (operation, payload = {}) => {
    if (!lease) throw new Error("Pi background host is not subscribed");
    sequence += 1;
    return request(operation, {
      ...payload,
      leaseId: lease.leaseId,
      activationId: lease.activationId,
      sequence,
    });
  };

  const taskFields = (view) => ({
    id: view.task.id,
    kind: view.kind,
    description: view.task.description?.slice(0, 512),
    command: view.task.command?.slice(0, 512),
    childSessionId: view.task.delegate?.childSessionId,
    canStop: hostOperations.has("stop") && view.task.status === "running",
    canReadOutput: hostOperations.has("detail") && (hostOperations.has("logs") || (hostOperations.has("result") && view.canReadResult)),
    status: view.task.status,
    notifyOnCompletion: view.task.notifyOnCompletion === true,
    notified: view.task.notified === true,
    triggerOnCompletion: view.task.triggerOnCompletion === true,
  });

  const readHeader = async (path, whole = false) => {
    const stat = await lstat(path);
    if (!stat.isFile() || stat.isSymbolicLink() || (whole && stat.size > 65536)) throw new Error("Not a bounded regular task artifact");
    const file = await open(path, "r");
    try {
      const buffer = Buffer.alloc(65536);
      const {bytesRead} = await file.read(buffer, 0, buffer.length, 0);
      const text = buffer.subarray(0, bytesRead).toString("utf8");
      if (!whole && !text.includes("\n")) throw new Error("Incomplete task session header");
      return JSON.parse(whole ? text : text.split("\n", 1)[0]);
    } finally { await file.close(); }
  };
  const delegateSession = async (view, sessionId, ctx) => {
    const facts = view.task.delegate;
    if (view.kind !== "delegate" || !facts || facts.taskId !== view.task.id || !isAbsolute(facts.artifactDirAbs ?? "")
        || sessionId !== ctx.sessionManager.getSessionId?.()) return undefined;
    try {
      const directory = join(facts.artifactDirAbs, "child-session");
      for (const path of [facts.artifactDirAbs, directory]) {
        const stat = await lstat(path);
        if (!stat.isDirectory() || stat.isSymbolicLink()) return undefined;
      }
      const manifest = await readHeader(join(facts.artifactDirAbs, "manifest.json"), true);
      if (manifest.task_id !== view.task.id || manifest.launch_nonce !== facts.launchNonce
          || manifest.child_session_id !== facts.childSessionId || manifest.child_session_dir !== directory
          || manifest.seed_sha256 !== facts.seedSha256) return undefined;
      const sessions = [];
      for (const entry of await readdir(directory, {withFileTypes:true})) {
        if (!entry.isFile() || !entry.name.endsWith(".jsonl")) continue;
        const path = join(directory, entry.name);
        const header = await readHeader(path);
        if (header.type === "session" && header.id === facts.childSessionId
            && (!header.parentSession || header.parentSession === ctx.sessionManager.getSessionFile())) sessions.push(path);
      }
      return sessions.length === 1 ? sessions[0] : undefined;
    } catch { return undefined; }
  };

  const readTasks = async (ctx) => {
    if (disposed || !lease) return;
    const tasks = [];
    let cursor;
    let revision = 0;
    let activationId = lease.activationId;
    do {
      const result = await leasedRequest("list", { limit: 50, ...(cursor ? { cursor } : {}) });
      if (disposed) return;
      if (result.activationId !== lease.activationId) throw new Error("Pi background host activation changed while listing tasks");
      const expectedSession = ctx.sessionManager.getSessionId?.();
      if (typeof expectedSession === "string" && result.sessionId !== expectedSession) throw new Error("Pi background task snapshot belongs to another session");
      activationId = result.activationId;
      revision = result.revision;
      if (Array.isArray(result.tasks)) for (const view of result.tasks) {
        tasks.push({...taskFields(view), childSessionFile:await delegateSession(view, result.sessionId, ctx)});
      }
      cursor = result.nextCursor ?? undefined;
    } while (cursor);
    publish(ctx, {
      version: FRAME_VERSION,
      kind: "snapshot",
      supported: true,
      activationId,
      revision,
      tasks,
    });
  };

  const refresh = async (ctx, force = false) => {
    if (disposed) return;
    if (refreshPromise) {
      refreshAgain = true;
      return refreshPromise;
    }
    if (retryTimer) {
      if (!force) return;
      clearTimeout(retryTimer);
      retryTimer = undefined;
    }
    refreshPromise = (async () => {
      do {
        if (disposed) break;
        refreshAgain = false;
        try {
          await readTasks(ctx);
          refreshFailures = 0;
        } catch (error) {
          if (disposed) break;
          refreshFailures += 1;
          publish(ctx, {
            version: FRAME_VERSION,
            kind: "unknown",
            supported: true,
            error: compactError(error),
            tasks: lastFrame.kind === "snapshot" ? lastFrame.tasks : [],
            activationId: lease?.activationId,
            revision: lastFrame.revision,
          });
          if (!retryTimer) {
            const delay = Math.min(5000, 250 * (2 ** Math.min(refreshFailures - 1, 4)));
            retryTimer = setTimeout(() => {
              retryTimer = undefined;
              if (disposed) return;
              void refresh(latestContext);
            }, delay);
            retryTimer.unref?.();
          }
          break;
        }
      } while (refreshAgain);
    })().finally(() => {
      refreshPromise = undefined;
    });
    return refreshPromise;
  };

  const scheduleSubscribeRetry = (ctx) => {
    if (disposed) return;
    refreshFailures += 1;
    if (retryTimer) return;
    const delay = Math.min(5000, 250 * (2 ** Math.min(refreshFailures - 1, 4)));
    retryTimer = setTimeout(() => {
      retryTimer = undefined;
      if (disposed) return;
      void ensureSubscribed(latestContext).then((supported) => supported && refresh(latestContext, true));
    }, delay);
    retryTimer.unref?.();
  };

  const ensureSubscribed = async (ctx) => {
    if (disposed) return false;
    latestContext = ctx;
    if (host === "unsupported") {
      publish(ctx, { version: FRAME_VERSION, kind: "unsupported", supported: false, tasks: [] });
      return false;
    }
    if (!host) {
      try {
        const caps = await request("capabilities", {});
        if (disposed) return false;
        if (caps.api_version !== 1 || !caps.operations?.includes("subscribe") || !caps.operations?.includes("list")) {
          host = "unsupported";
          publish(ctx, { version: FRAME_VERSION, kind: "unsupported", supported: false, tasks: [] });
          return false;
        }
        host = "available";
        hostOperations = new Set(caps.operations);
      } catch (error) {
        if (disposed) return false;
        if (!hostObserved && error?.code === "background_host_timeout") {
          // No responder was observed. A missing optional plugin is a stable
          // capability result; an actual host rejection remains unknown.
          host = "unsupported";
          publish(ctx, { version: FRAME_VERSION, kind: "unsupported", supported: false, tasks: [] });
          return false;
        }
        publish(ctx, { version: FRAME_VERSION, kind: "unknown", supported: true, error: compactError(error), tasks: [] });
        scheduleSubscribeRetry(ctx);
        return true;
      }
    }
    if (!lease) {
      try {
        const subscribed = await request("subscribe", { clientId: CLIENT_ID });
        if (disposed) return false;
        lease = { leaseId: subscribed.leaseId, activationId: subscribed.activationId };
        sequence = 0;
      } catch (error) {
        if (disposed) return false;
        publish(ctx, { version: FRAME_VERSION, kind: "unknown", supported: true, error: compactError(error), tasks: [] });
        scheduleSubscribeRetry(ctx);
        return true;
      }
    }
    return true;
  };

  removeChangedListener = pi.events.on(CHANGED, (frame) => {
    if (disposed) return;
    if (!lease || frame?.schema_version !== "pi-background-tasks.host-changed.v1") return;
    if (frame.activationId !== lease.activationId) {
      lease = undefined;
      sequence = 0;
      void ensureSubscribed(latestContext).then((supported) => !disposed && supported && refresh(latestContext, true));
      return;
    }
    if (frame.kind === "closed") {
      publish(latestContext, { version: FRAME_VERSION, kind: "unknown", supported: true, error: "Pi background host activation closed", tasks: lastFrame.tasks, activationId: lease.activationId, revision: frame.revision });
      return;
    }
    void refresh(latestContext);
  });

  pi.registerCommand("velune-background-work", {
    description: "Synchronize Pi background task state for Velune",
    handler: async (args, ctx) => {
      if (disposed) return;
      latestContext = ctx;
      useChildParent(ctx.sessionManager.getSessionFile());
      let command;
      try {
        command = JSON.parse(Buffer.from(args.trim(), "base64url").toString("utf8"));
      } catch {
        throw new Error("Invalid Velune background host command");
      }
      try {
      // These are independent optional extension protocols. Serial cold
      // discovery takes two 1.5s absence timeouts and races the 3s RPC deadline.
      const [, supported] = await Promise.all([
        readChildren(ctx),
        command.operation === "stop_child" ? false : ensureSubscribed(ctx),
      ]);
      if (command.operation === "stop_child") {
        const matches = children.filter(child => child.sessionFile === command.sessionFile && child.canStop);
        if (childrenUnknown || matches.length !== 1) {
          publish(ctx, lastFrame);
          throw new Error("Pi child cannot be stopped independently");
        }
        await subagentRpc("stop", {id:matches[0].runId});
        children = children.map(child => child.sessionFile === command.sessionFile ? {...child, state:"stopping", canStop:false} : child);
        publish(ctx, lastFrame);
        return;
      }
      if (command.operation === "stop_all") {
        if (childrenUnknown) throw new Error("Pi subagent state is unknown");
        for (const id of new Set(children.map(child => child.runId))) await subagentRpc("stop", {id});
        children = children.map(child => ({...child, state:"stopping", canStop:false}));
      }
      if (disposed) return;
      if (!supported) return publish(ctx, lastFrame);
      if (!lease) return publish(ctx, lastFrame);
      if (command.operation === "task_output") {
        const owner = ctx.sessionManager.getSessionFile?.();
        let output;
        try {
          if (!lastFrame.tasks?.some(task => task.id === command.taskId && task.canReadOutput)) throw new Error("Task output is not available");
          const detail = await leasedRequest("detail", {taskId:command.taskId});
          if (detail.task?.task?.id !== command.taskId) throw new Error("Pi background task detail identity mismatch");
          if (detail.task.canReadResult && hostOperations.has("result")) {
            const result = await leasedRequest("result", {taskId:command.taskId});
            if (result.taskId !== command.taskId) throw new Error("Pi background task result identity mismatch");
            output = {text:result.text ?? result.message ?? (result.delivery === "artifact" ? "结果已提交，内容超过内联显示上限。" : "任务尚未提供结果。"), truncated:result.delivery === "artifact"};
          } else {
            const result = await leasedRequest("logs", {taskId:command.taskId,maxBytes:65536,tail:true});
            if (result.task?.id !== command.taskId) throw new Error("Pi background task logs identity mismatch");
            output = {text:result.text, truncated:result.truncated};
          }
        } catch (error) { output = {error:compactError(error)}; }
        if (disposed || latestContext?.sessionManager.getSessionFile?.() !== owner) return;
        if (typeof output.text === "string") {
          const bounded = boundedOutput(output.text);
          output = {...output,text:bounded.text,truncated:output.truncated || bounded.truncated};
        }
        publish(ctx, {...lastFrame,taskOutput:{requestId:command.requestId,taskId:command.taskId,...output}});
        return;
      }
      if (command.operation === "stop_task") {
        const owner = ctx.sessionManager.getSessionFile?.();
        try {
          if (!lastFrame.tasks?.some(task => task.id === command.taskId && task.canStop)) throw new Error("Task cannot be stopped independently");
          const stopped = await leasedRequest("stop", {taskId:command.taskId});
          if (stopped.task?.id !== command.taskId) throw new Error("Pi background task stop identity mismatch");
          await refresh(ctx, true);
          if (disposed || latestContext?.sessionManager.getSessionFile?.() !== owner) return;
          publish(ctx, {...lastFrame,taskOperation:{taskId:command.taskId}});
        } catch (error) {
          if (disposed || latestContext?.sessionManager.getSessionFile?.() !== owner) return;
          publish(ctx, {...lastFrame,taskOperation:{taskId:command.taskId,error:compactError(error)}});
        }
        return;
      }
      if (command.operation === "stop_all") {
        const taskIds = [...new Set(command.taskIds ?? [])].filter((id) => typeof id === "string" && id.length > 0);
        if (taskIds.length > 0) {
          const results = [];
          for (let index = 0; index < taskIds.length; index += 1000) {
            const stopped = await leasedRequest("stop_all", { taskIds: taskIds.slice(index, index + 1000) });
            if (Array.isArray(stopped.results)) results.push(...stopped.results);
          }
          await refresh(ctx, true);
          const stopResults = results.map(({ taskId, outcome, status }) => ({ taskId, outcome, status }));
          publish(ctx, { ...lastFrame, stopResults });
        }
      } else {
        await refresh(ctx, true);
      }
      publish(ctx, lastFrame);
      } catch (error) {
        if (!disposed) throw error;
      }
    },
  });

  const refreshChildren = () => {
    if (disposed || !latestContext || subagentSupported === false) return;
    const ctx = latestContext, parent = ctx.sessionManager.getSessionFile();
    const reading = readChildren(ctx), sequence = childReadSequence;
    void reading.then(() => {
      if (!disposed && sequence === childReadSequence && parent === latestContext?.sessionManager.getSessionFile()) publish(ctx, lastFrame);
    });
  };
  const childPoll = setInterval(refreshChildren, 2000);
  childPoll.unref?.();
  const removeStartedListener = pi.events.on("subagent:async-started", () => {
    refreshChildren();
  });
  const removeChildListener = pi.events.on("subagent:child-status", () => {
    refreshChildren();
  });
  const removeCompleteListener = pi.events.on("subagent:async-complete", () => {
    refreshChildren();
  });
  removeSessionStartListener = pi.on("session_start", (_event, ctx) => {
    if (!disposed) {
      latestContext = ctx;
      useChildParent(ctx.sessionManager.getSessionFile());
    }
  });

  removeShutdownListener = pi.on("session_shutdown", () => {
    if (disposed) return;
    disposed = true;
    latestContext = undefined;
    lease = undefined;
    refreshAgain = false;
    if (retryTimer) {
      clearTimeout(retryTimer);
      retryTimer = undefined;
    }
    removeResponseListener();
    removeChangedListener();
    removeChildListener();
    removeCompleteListener();
    removeStartedListener();
    clearInterval(childPoll);
    removeSessionStartListener();
    for (const item of pending.values()) {
      clearTimeout(item.timer);
      item.reject(new Error("Pi background host activation is closed"));
    }
    pending.clear();
    // This handler is already running; removing it prevents retention by the
    // Pi event runner if it keeps the old extension object through teardown.
    removeShutdownListener();
  });
}
