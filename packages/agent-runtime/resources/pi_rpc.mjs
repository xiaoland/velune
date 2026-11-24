// Fixed Pi SDK assembly: source PI_HOME owns its settings, credentials and sessions;
// Velune appends its current gateway provider catalog for this execution.
import { readFileSync } from "node:fs";
import { dirname, isAbsolute, join } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { resolvePiSdk } from "./pi_sdk.mjs";
const args = process.argv.slice(2);
const value = (key) => { const index = args.indexOf(key); return index < 0 ? undefined : args[index + 1]; };
const fail = (message) => { throw new Error(message); };
// Resolve the optional integration from the extension which Pi actually loaded,
// not the app's dependency tree or an unrelated global npm installation.
async function childExtensionRegistration(resourceLoader) {
  for (const loaded of resourceLoader.getExtensions().extensions) {
    if (typeof loaded.resolvedPath !== "string" || !isAbsolute(loaded.resolvedPath)) continue;
    let directory = dirname(loaded.resolvedPath);
    for (;;) {
      let manifest;
      try { manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")); }
      catch (error) { if (error.code !== "ENOENT") break; }
      if (manifest) {
        if (manifest.name !== "pi-subagents") break;
        try {
          const require = createRequire(join(directory, "package.json"));
          const api = await import(pathToFileURL(require.resolve("pi-subagents/required-child-extensions")).href);
          if (typeof api.registerRequiredChildExtensions === "function") return api.registerRequiredChildExtensions;
        } catch { /* Older optional plugins do not expose this public API. */ }
        return undefined;
      }
      const parent = dirname(directory);
      if (parent === directory) break;
      directory = parent;
    }
  }
}
async function main() {
  const cli = value("--cli");
  if (!cli || !isAbsolute(cli)) fail("Pi CLI 必须是绝对路径");
  const installation = resolvePiSdk({binary:cli,runtimeTypeId:value("--runtime-type")});
  // pi-subagents cannot infer Pi's host from this Node SDK entrypoint. Its
  // public override also keeps detached runners on the selected installation.
  process.env.PI_SUBAGENTS_PI_CODING_AGENT_PACKAGE_ROOT = installation.packageDirectory;
  // Extensions need Pi's own entry converter to build display history. Bind it
  // to this validated installation rather than resolving another global SDK.
  process.env.VELUNE_PI_SDK_PACKAGE_ROOT = installation.packageDirectory;
  if (args.includes("--validate")) return;
  const required = (key) => { const path = value(key); if (!path || !isAbsolute(path)) fail("Pi SDK 装配路径无效"); return path; };
  const agentDir = required("--agent-dir"), modelsPath = value("--models-path"), selectionFile = value("--selection-file"), extension = required("--extension");
  process.env.PI_CODING_AGENT_DIR = agentDir;
  if (selectionFile) process.env.VELUNE_PI_SELECTION_FILE = selectionFile;
  else delete process.env.VELUNE_PI_SELECTION_FILE;
  if (modelsPath) process.env.VELUNE_PI_MODELS_PATH = modelsPath;
  else delete process.env.VELUNE_PI_MODELS_PATH;
  const sdk = await installation.importSdk();
  const compactionReserves = new Map(Object.entries(JSON.parse(process.env.VELUNE_PI_COMPACTION_RESERVES ?? "{}")));
  delete process.env.VELUNE_PI_COMPACTION_RESERVES;
  const configuredReserve = model => model ? compactionReserves.get(`${model.provider}/${model.id}`) : undefined;
  const validateReserve = model => {
    const reserve = configuredReserve(model);
    const window = model?.contextWindow;
    if (reserve !== undefined && Number.isFinite(window) && window > 0 && reserve >= window) {
      fail(`Pi 自动压缩预留量 ${reserve} 必须小于所选模型的上下文窗口 ${window}`);
    }
  };
  const injectedMcp = JSON.parse(process.env.VELUNE_MCP_SERVERS ?? "{}");
  delete process.env.VELUNE_MCP_SERVERS;
  const mcpNames = Object.keys(injectedMcp);
  const { resolveProjectTrusted } = await installation.importModule("dist/core/project-trust.js");
  const { createProjectTrustContext } = await installation.importModule("dist/cli/project-trust.js");
  const sourceModelsPath = join(agentDir,"models.json");
  const modelRuntime = await sdk.ModelRuntime.create({authPath:join(agentDir,"auth.json"),modelsPath:sourceModelsPath});
  const modelRuntimeError = modelRuntime.getError();
  if (modelRuntimeError) fail(`Pi 源模型目录无效（${sourceModelsPath}）：${modelRuntimeError}`);
  const gatewayCatalog = modelsPath ? JSON.parse(readFileSync(modelsPath,"utf8")) : {};
  for (const [providerId, provider] of Object.entries(gatewayCatalog.providers ?? {})) {
    modelRuntime.registerProvider(providerId, provider);
  }
  const sessionPath = value("--session"), sessionDir = value("--session-dir");
  // A side chat runs on an in-memory copy of the session that is never saved.
  const sessionManager = args.includes("--side")
    ? sdk.SessionManager.inMemory(process.cwd(), undefined, sessionPath ? sdk.parseSessionEntries(readFileSync(sessionPath, "utf8")) : undefined)
    : sessionPath ? sdk.SessionManager.open(sessionPath, sessionDir) : sdk.SessionManager.create(process.cwd(), sessionDir);
  const trustStore = new sdk.ProjectTrustStore(agentDir), trustByCwd = new Map();
  const factory = async ({cwd,agentDir,sessionManager,sessionStartEvent,projectTrustContext}) => {
    let registerChildExtensions;
    let childExtensions;
    const childExtensionHost = pi => {
      pi.on("session_start", (_event, ctx) => {
        childExtensions?.dispose(); childExtensions = undefined;
        if (!selectionFile) return;
        if (!registerChildExtensions) {
          ctx.ui.setStatus("velune.child-capabilities", JSON.stringify({requiredExtensions:"unavailable"}));
          return;
        }
        try {
          childExtensions = registerChildExtensions({sessionId:ctx.sessionManager.getSessionId(),
            extensions:[{id:"velune.virtual-model",path:extension}]});
          ctx.ui.setStatus("velune.child-capabilities", JSON.stringify({requiredExtensions:"supported"}));
        } catch (error) {
          // Another host owns its registration, or this plugin is incompatible.
          // Do not dispose its policy or prevent ordinary Pi from starting.
          ctx.ui.setStatus("velune.child-capabilities", JSON.stringify({requiredExtensions:"unavailable",
            detail:error instanceof Error ? error.message : "Required child extension registration failed"}));
        }
      });
      pi.on("session_shutdown", () => { childExtensions?.dispose(); childExtensions = undefined; });
    };
    const hasProjectResources = sdk.hasTrustRequiringProjectResources(cwd);
    const cached = trustByCwd.get(cwd);
    const shouldResolve = cached === undefined && hasProjectResources;
    const projectTrusted = shouldResolve ? false : (cached ?? (!hasProjectResources || trustStore.get(cwd) === true));
    const settingsManager = sdk.SettingsManager.create(cwd,agentDir,{projectTrusted});
    if (compactionReserves.size) {
      // Resolve through the current model on each check. Pi reloads its merged
      // settings on trust changes and writes, so a one-time settings merge would be lost.
      const reserveFor = settingsManager.getCompactionReserveTokens.bind(settingsManager);
      settingsManager.getCompactionReserveTokens = model => configuredReserve(model) ?? reserveFor(model);
    }
    const services = await sdk.createAgentSessionServices({cwd,agentDir,settingsManager,modelRuntime,
      resourceLoaderOptions:{additionalExtensionPaths:[extension],extensionFactories:[childExtensionHost,...(mcpNames.length ? [
        sdk.createMcpExtension(),
        pi => {
          for (const [name, config] of Object.entries(injectedMcp)) pi.registerMcpServer(name,{...config,exposure:"direct"});
        },
      ] : [])]},
      resourceLoaderReloadOptions:shouldResolve ? {resolveProjectTrust:async ({extensionsResult})=> {
        const trusted = await resolveProjectTrusted({cwd,trustStore,defaultProjectTrust:settingsManager.getDefaultProjectTrust(),extensionsResult,
          projectTrustContext:projectTrustContext ?? createProjectTrustContext({cwd,mode:"rpc",settingsManager,hasUI:false})});
        trustByCwd.set(cwd,trusted); return trusted;
      }} : undefined,
    });
    registerChildExtensions = selectionFile ? await childExtensionRegistration(services.resourceLoader) : undefined;
    const provider = value("--provider"), modelId = value("--model");
    const model = provider && modelId ? services.modelRuntime.getModel(provider,modelId) : undefined;
    if (provider && modelId && !model) fail("所选运行时模型未注册");
    validateReserve(model);
    const result = await sdk.createAgentSessionFromServices({services,sessionManager,model,thinkingLevel:"off",sessionStartEvent});
    if (result.session.model !== model) validateReserve(result.session.model);
    if (compactionReserves.size) {
      const setModel = result.session.setModel.bind(result.session);
      result.session.setModel = async (nextModel, options) => {
        validateReserve(nextModel);
        return setModel(nextModel, options);
      };
    }
    return {...result,services,diagnostics:services.diagnostics};
  };
  const host = await sdk.createAgentSessionRuntime(factory,{cwd:sessionManager.getCwd(),agentDir,sessionManager});
  await sdk.runRpcMode(host);
}
main().catch((error)=>{ process.stderr.write(`${error instanceof Error ? error.message : "Pi SDK 装配失败"}\n`); process.exitCode=1; });
