// Velune's single public Pi model. The application owns the physical route;
// Pi only records this virtual selection and the routed model in its branch.
import { readFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import installPiBackgroundWork from "./pi_background_work.mjs";
import { projectPiDisplayHistory } from "./pi_transcript.mjs";

const sdkPackageRoot = process.env.VELUNE_PI_SDK_PACKAGE_ROOT;
const sessionManagerSdk = sdkPackageRoot
  ? await import(pathToFileURL(join(sdkPackageRoot, "dist/core/session-manager.js")).href)
  : undefined;

const selectionPath = process.env.VELUNE_PI_SELECTION_FILE;

function selection() {
  if (!selectionPath) throw new Error("Velune Pi selection file is missing");
  const value = JSON.parse(readFileSync(selectionPath, "utf8"));
  if (!value.modelRecordKey || !value.physicalModelId) throw new Error("Velune Pi selection model is missing");
  return value;
}

export default function (pi) {
  installPiBackgroundWork(pi);
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
      if (!sessionManagerSdk?.sessionEntryToContextMessages) {
        throw new Error("Validated Pi SDK entry converter is unavailable");
      }
      // One synchronous projection supplies both content and provenance. A
      // later get_messages reads agent.state.messages, which can already be a
      // different branch or include an in-flight message not persisted here.
      const projection = projectPiDisplayHistory(manager, sessionManagerSdk.sessionEntryToContextMessages);
      const sessionId = manager.getSessionId();
      const messages = [];
      const messageIds = [];
      const identityConfirmations = [];
      for (const entry of projection.entries) {
        for (const [ordinal, message] of entry.messages.entries()) {
          const currentId = `pi:${sessionId}:${entry.sourceEntry.id}:${ordinal}`;
          messages.push({...message, id:currentId});
          messageIds.push(currentId);
          const previousId = entry.sourceEntry.type === "message"
            ? liveMessages.get(entry.sourceEntry.message) : undefined;
          if (previousId && ordinal === 0) identityConfirmations.push({previousId, currentId});
        }
      }
      ctx.ui.setStatus("velune.session-metadata", JSON.stringify({sessionFile:manager.getSessionFile() ?? null,cwd:manager.getCwd(),name:manager.getSessionName() ?? null,createdUnixMs:Date.parse(manager.getHeader()?.timestamp ?? ""),requestId:_args.trim(),snapshotVersion:1,messages,messageIds,identityConfirmations}));
    },
  });
  // Editing a user message in place: the leaf moves to just before the
  // message's entry, in the same session file, so the next prompt replaces
  // it; the earlier branch stays in Pi's tree. Reports its outcome as a status.
  pi.registerCommand("velune-rewind", {
    description: "Move the session leaf to before a user message",
    handler: async (args, ctx) => {
      const entryId = args.trim();
      let outcome;
      try {
        const entry = ctx.sessionManager.getEntry(entryId);
        if (entry?.type !== "message" || entry.message.role !== "user") throw new Error("not a user message");
        const branch = ctx.sessionManager.getBranch();
        const boundary = branch.findIndex(item => item.id === entryId);
        if (boundary < 0) throw new Error("user message is not on the active branch");
        const prefix = new Set(branch.slice(0, boundary).map(item => item.id));
        // Pi treats navigation to the current leaf as a no-op, even when that
        // leaf is a user message. Its tree navigation also omits later edits
        // to earlier entries. Neither can faithfully replace this boundary;
        // leave the source untouched and use the application's native rebuild.
        if (ctx.sessionManager.getLeafId() === entryId
            || branch.slice(boundary + 1).some(item => item.type === "context_edit" && prefix.has(item.targetId))) {
          outcome = {entryId, fallback:"Pi tree navigation cannot preserve this edit boundary",
            unmodified:true, reason:ctx.sessionManager.getLeafId() === entryId ? "user_leaf" : "prefix_context_edit"};
        } else {
          const result = await ctx.navigateTree(entryId, {summarize:false});
          if (result.cancelled) outcome = {error:"cancelled"};
          else if (ctx.sessionManager.getLeafId() !== entry.parentId) outcome = {error:"Pi did not move to the requested parent"};
          else outcome = {entryId};
        }
      } catch (error) {
        outcome = {error: error instanceof Error ? error.message : "rewind failed"};
      }
      ctx.ui.setStatus("velune.rewind", JSON.stringify(outcome));
    },
  });
  if (!selectionPath) return;
  // Each session keeps its launch route. Foreground children share the Node
  // process and detached children inherit the parent's selection-file path;
  // neither may follow a later model change in the parent's ordinary composer.
  let chosen = selection();
  const appendSelection = () => {
    pi.appendEntry("pi.virtual-model-state", {
      provider: "velune-gateway",
      modelId: chosen.physicalModelId,
      state: {
        modelRecordKey: chosen.modelRecordKey,
        physicalModelId: chosen.physicalModelId,
        modelId: chosen.physicalModelId,
        thinkingLevel: pi.getThinkingLevel(),
      },
    });
  };
  pi.registerCommand("velune-sync-selection", {
    description: "Persist the Velune physical model selection on this branch",
    handler: () => { chosen = selection(); appendSelection(); },
  });
  pi.on("model_select", (event) => {
    if (event.model?.provider === "velune" && event.model.id === "auto") appendSelection();
  });
  // Extension admission (including delegates) reads catalog limits before
  // making a route request. A virtual model's default limits are zero.
  const catalog = JSON.parse(readFileSync(process.env.VELUNE_PI_MODELS_PATH, "utf8"));
  // Detached children have their own ModelRuntime: the parent's in-memory
  // provider registration is not inherited. Keep the required route together
  // with its virtual model, without changing the user's models.json.
  pi.registerProvider("velune-gateway", catalog.providers["velune-gateway"]);
  const physical = catalog.providers["velune-gateway"].models.find(model => model.id === chosen.physicalModelId);
  if (!physical) throw new Error("Velune Pi selected model is absent from its catalog");
  pi.registerVirtualModel({
    provider: "velune",
    id: "auto",
    name: "Velune Auto",
    contextWindow: physical.contextWindow,
    maxTokens: physical.maxTokens,
    thinkingLevels: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    input: ["text", "image"],
    route(request, ctx) {
      const physicalModelId = chosen.physicalModelId;
      const model = ctx.modelRegistry.find("velune-gateway", physicalModelId);
      if (!model) throw new Error(`Velune gateway model is unavailable: ${physicalModelId}`);
      // The session's level, which Velune sets per turn through RPC.
      const thinkingLevel = request.thinkingLevel ?? "off";
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
