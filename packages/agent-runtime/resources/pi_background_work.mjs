// Structured bridge to pi-background-tasks' public Host EventBus API v1.
// The RPC stdin stream stays owned by Pi's SDK; requests cross it only as
// ordinary Velune extension commands and versioned setStatus frames.
import { randomUUID } from "node:crypto";

const REQUEST = "pi-background-tasks:host:request:v1";
const RESPONSE = "pi-background-tasks:host:response:v1";
const CHANGED = "pi-background-tasks:host:changed:v1";
const CLIENT_ID = "velune-pi-runtime-v1";
const STATUS_KEY = "velune.background-work-v1";
const FRAME_VERSION = 1;
const REQUEST_TIMEOUT_MS = 1500;

const compactError = (error) => {
  const text = error instanceof Error ? error.message : String(error);
  return text.replace(/\s+/gu, " ").trim().slice(0, 512) || "unknown host API error";
};

export default function installPiBackgroundWork(pi) {
  const pending = new Map();
  let latestContext;
  let disposed = false;
  let host;
  let lease;
  let sequence = 0;
  let refreshPromise;
  let refreshAgain = false;
  let refreshFailures = 0;
  let retryTimer;
  let lastFrame = { version: FRAME_VERSION, kind: "uninitialized", tasks: [] };
  let removeResponseListener = () => {};
  let removeChangedListener = () => {};
  let removeSessionStartListener = () => {};
  let removeShutdownListener = () => {};

  const publish = (ctx, frame) => {
    if (disposed) return;
    if (ctx) latestContext = ctx;
    if (!latestContext) return;
    lastFrame = frame;
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
      reject(new Error(`Pi background host ${operation} request timed out`));
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
    status: view.task.status,
    notifyOnCompletion: view.task.notifyOnCompletion === true,
    notified: view.task.notified === true,
    triggerOnCompletion: view.task.triggerOnCompletion === true,
  });

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
      activationId = result.activationId;
      revision = result.revision;
      if (Array.isArray(result.tasks)) tasks.push(...result.tasks.map(taskFields));
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
      } catch (_error) {
        if (disposed) return false;
        // Missing plugin is a stable capability result. Once capabilities
        // succeeded, later errors remain unknown and keep the process alive.
        host = "unsupported";
        publish(ctx, { version: FRAME_VERSION, kind: "unsupported", supported: false, tasks: [] });
        return false;
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
      let command;
      try {
        command = JSON.parse(Buffer.from(args.trim(), "base64url").toString("utf8"));
      } catch {
        throw new Error("Invalid Velune background host command");
      }
      try {
      const supported = await ensureSubscribed(ctx);
      if (disposed) return;
      if (!supported) return publish(ctx, lastFrame);
      if (!lease) return publish(ctx, lastFrame);
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

  removeSessionStartListener = pi.on("session_start", (_event, ctx) => {
    if (!disposed) latestContext = ctx;
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
