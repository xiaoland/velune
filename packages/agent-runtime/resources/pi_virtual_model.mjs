// Velune's single public Pi model. The application owns the physical route;
// Pi only records this virtual selection and the routed model in its branch.
import { readFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const selectionPath = process.env.VELUNE_PI_SELECTION_FILE;

function selection() {
  if (!selectionPath) throw new Error("Velune Pi selection file is missing");
  const value = JSON.parse(readFileSync(selectionPath, "utf8"));
  if (!value.modelRecordKey || !value.physicalModelId) throw new Error("Velune Pi selection model is missing");
  return value;
}

export default function (pi) {
  // RPC emits message_end before SessionManager appends its entry. Keep only
  // object-reference associations until the native entry supplies its identity.
  const liveMessages = new WeakMap();
  let activeMessageIdentity;
  pi.on("message_start", (_event, ctx) => {
    activeMessageIdentity = `pi-live:${randomUUID()}`;
    ctx.ui.setStatus("velune.message-identity", activeMessageIdentity);
  });
  pi.on("message_end", (event) => {
    if (activeMessageIdentity) liveMessages.set(event.message, activeMessageIdentity);
    activeMessageIdentity = undefined;
  });
  // get_state omits cwd. Query the current public extension context at every
  // authoritative resync, including in-place branch/session changes.
  pi.registerCommand("velune-projection-sync", {
    description: "Project current native session metadata",
    handler: (_args, ctx) => {
      const manager = ctx.sessionManager;
      const messageIds = [];
      const identityConfirmations = [];
      for (const entry of manager.buildSessionProjection().entries) {
        for (const [ordinal] of entry.messages.entries()) {
          const currentId = `pi:${manager.getSessionId()}:${entry.sourceEntry.id}:${ordinal}`;
          messageIds.push(currentId);
          const previousId = entry.sourceEntry.type === "message"
            ? liveMessages.get(entry.sourceEntry.message) : undefined;
          if (previousId && ordinal === 0) identityConfirmations.push({previousId, currentId});
        }
      }
      ctx.ui.setStatus("velune.session-metadata", JSON.stringify({sessionFile:manager.getSessionFile() ?? null,cwd:manager.getCwd(),name:manager.getSessionName() ?? null,createdUnixMs:Date.parse(manager.getHeader()?.timestamp ?? ""),messageIds,identityConfirmations}));
    },
  });
  const appendSelection = () => {
    const chosen = selection();
    pi.appendEntry("pi.virtual-model-state", {
      provider: "velune-gateway",
      modelId: chosen.physicalModelId,
      state: {
        modelRecordKey: chosen.modelRecordKey,
        physicalModelId: chosen.physicalModelId,
        modelId: chosen.physicalModelId,
        thinkingLevel: chosen.thinkingLevel ?? "off",
      },
    });
  };
  pi.registerCommand("velune-sync-selection", {
    description: "Persist the Velune physical model selection on this branch",
    handler: appendSelection,
  });
  pi.on("model_select", () => {
    appendSelection();
  });
  // Velune's gateway decides subscription capabilities. Pi's generic
  // Responses adapter otherwise adds API-key-oriented generation and cache
  // fields based on local catalog metadata.
  pi.on("before_provider_request", (event) => {
    if (!event.payload || typeof event.payload !== "object") return event.payload;
    const chosen = selection();
    if (chosen.subscriptionCapability !== true) return event.payload;
    const payload = { ...event.payload };
    for (const key of [
      "max_output_tokens",
      "temperature",
      "prompt_cache_retention",
      "prompt_cache_options",
      "prompt_cache_key",
    ]) delete payload[key];
    return payload;
  });
  pi.registerVirtualModel({
    provider: "velune",
    id: "auto",
    name: "Velune Auto",
    thinkingLevels: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    input: ["text"],
    route(request, ctx) {
      const chosen = selection();
      const physicalModelId = chosen.physicalModelId;
      const model = ctx.modelRegistry.find("velune-gateway", physicalModelId);
      if (!model) throw new Error(`Velune gateway model is unavailable: ${physicalModelId}`);
      const thinkingLevel = chosen.thinkingLevel ?? request.thinkingLevel ?? "off";
      return {
        model,
        thinkingLevel,
        state: {
          provider: "velune-gateway",
          modelRecordKey: chosen.modelRecordKey,
          physicalModelId,
          modelId: physicalModelId,
          thinkingLevel,
        },
      };
    },
  });
}
