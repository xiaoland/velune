// Fixed Pi SDK assembly: source PI_HOME owns settings and sessions; Velune owns
// the injected catalog and ephemeral gateway credentials in a separate directory.
import { isAbsolute } from "node:path";
import { resolvePiSdk } from "./pi_sdk.mjs";
const args = process.argv.slice(2);
const value = (key) => { const index = args.indexOf(key); return index < 0 ? undefined : args[index + 1]; };
const fail = (message) => { throw new Error(message); };
async function main() {
  const cli = value("--cli");
  if (!cli || !isAbsolute(cli)) fail("Pi CLI 必须是绝对路径");
  const installation = resolvePiSdk({binary:cli,runtimeTypeId:value("--runtime-type")});
  if (args.includes("--validate")) return;
  const required = (key) => { const path = value(key); if (!path || !isAbsolute(path)) fail("Pi SDK 装配路径无效"); return path; };
  const agentDir = required("--agent-dir"), modelsPath = required("--models-path"), selectionFile = required("--selection-file"), extension = required("--extension");
  process.env.PI_CODING_AGENT_DIR = agentDir;
  process.env.VELUNE_PI_SELECTION_FILE = selectionFile;
  const sdk = await installation.importSdk();
  const { resolveProjectTrusted } = await installation.importModule("dist/core/project-trust.js");
  const { createProjectTrustContext } = await installation.importModule("dist/cli/project-trust.js");
  // Execution has no access to the source credential storage. The configured
  // catalog resolves only the application's ephemeral loopback token.
  const credentials = { read:async()=>undefined, list:async()=>[], modify:async()=>fail("执行运行时不能修改原认证来源"), delete:async()=>fail("执行运行时不能修改原认证来源") };
  const { InMemoryCodingAgentModelsStore } = await installation.importModule("dist/core/models-store.js");
  const modelRuntime = await sdk.ModelRuntime.create({modelsPath,credentials,modelsStore:new InMemoryCodingAgentModelsStore(),refreshOnCreate:false,allowModelNetwork:false});
  if (modelRuntime.getError()) fail("Velune 执行模型目录无效");
  const reservedProviderIds = new Set(["velune", "velune-gateway"]);
  let reservedProviderConflict;
  const providerRegistrationError = (providerId) => {
    reservedProviderConflict = providerId;
    return new Error(`Pi 扩展不能注册 Velune 保留提供商：${providerId}`);
  };
  const registerProvider = modelRuntime.registerProvider.bind(modelRuntime);
  modelRuntime.registerProvider = (...args) => {
    const providerId = typeof args[0] === "string" ? args[0] : args[0]?.id;
    if (reservedProviderIds.has(providerId)) throw providerRegistrationError(providerId);
    return registerProvider(...args);
  };
  const registerNativeProvider = modelRuntime.registerNativeProvider.bind(modelRuntime);
  modelRuntime.registerNativeProvider = (provider) => {
    if (reservedProviderIds.has(provider?.id)) throw providerRegistrationError(provider.id);
    return registerNativeProvider(provider);
  };
  const registerVirtualModel = modelRuntime.registerVirtualModel.bind(modelRuntime);
  modelRuntime.registerVirtualModel = (definition) => {
    const providerId = definition?.provider;
    const modelId = definition?.id;
    if (providerId === "velune-gateway" || (providerId === "velune" && modelId !== "auto")) {
      throw providerRegistrationError(`${providerId}/${modelId}`);
    }
    if (providerId === "velune" && modelId === "auto" && definition?.__veluneOwned !== true) {
      throw providerRegistrationError(`${providerId}/${modelId}`);
    }
    return registerVirtualModel(definition);
  };
  const injectedGatewayModels = new Map(
    modelRuntime.getAllModels("velune-gateway").map((model) => [model.id, {
      api: model.api,
      baseUrl: model.baseUrl,
    }]),
  );
  const streamSimple = modelRuntime.streamSimple.bind(modelRuntime);
  modelRuntime.streamSimple = (model, context, options) => {
    const isVeluneVirtualModel = model?.api === "pi-virtual" && model.provider === "velune" && model.id === "auto";
    const expected = injectedGatewayModels.get(model?.id);
    const isInjectedGatewayModel = expected !== undefined && model?.provider === "velune-gateway" && expected.api === model.api && expected.baseUrl === model.baseUrl;
    if (!isVeluneVirtualModel && !isInjectedGatewayModel) fail("Velune 会话请求必须使用注入网关");
    if (modelRuntime.getRegisteredProviderIds().some((id) => reservedProviderIds.has(id))) {
      fail("扩展覆盖了 Velune 保留提供商");
    }
    return streamSimple(model, context, options);
  };
  const sessionPath = value("--session"), sessionDir = value("--session-dir");
  const sessionManager = sessionPath ? sdk.SessionManager.open(sessionPath, sessionDir) : sdk.SessionManager.create(process.cwd(), sessionDir);
  const trustStore = new sdk.ProjectTrustStore(agentDir), trustByCwd = new Map();
  const factory = async ({cwd,agentDir,sessionManager,sessionStartEvent,projectTrustContext}) => {
    const hasProjectResources = sdk.hasTrustRequiringProjectResources(cwd);
    const cached = trustByCwd.get(cwd);
    const shouldResolve = cached === undefined && hasProjectResources;
    const projectTrusted = shouldResolve ? false : (cached ?? (!hasProjectResources || trustStore.get(cwd) === true));
    const settingsManager = sdk.SettingsManager.create(cwd,agentDir,{projectTrusted});
    reservedProviderConflict = undefined;
    const services = await sdk.createAgentSessionServices({cwd,agentDir,settingsManager,modelRuntime,
      resourceLoaderOptions:{additionalExtensionPaths:[extension]},
      resourceLoaderReloadOptions:shouldResolve ? {resolveProjectTrust:async ({extensionsResult})=> {
        const trusted = await resolveProjectTrusted({cwd,trustStore,defaultProjectTrust:settingsManager.getDefaultProjectTrust(),extensionsResult,
          projectTrustContext:projectTrustContext ?? createProjectTrustContext({cwd,mode:"rpc",settingsManager,hasUI:false})});
        trustByCwd.set(cwd,trusted); return trusted;
      }} : undefined,
    });
    if (reservedProviderConflict) fail(`Pi 扩展不能注册 Velune 保留提供商：${reservedProviderConflict}`);
    // Existing Pi extensions may register unrelated source providers while the
    // source runtime loads. The application selects velune/auto at session
    // creation and rebinds it before every user turn; Pi's virtual-model
    // stream path then resolves only the injected velune-gateway catalog.
    const model = services.modelRuntime.getModel("velune","auto");
    if (!model) fail("Velune 网关模型未注册");
    if (model.api !== "pi-virtual") fail("Velune 网关模型命名空间被其他 Pi 模型占用");
    const result = await sdk.createAgentSessionFromServices({services,sessionManager,model,thinkingLevel:"off",sessionStartEvent});
    return {...result,services,diagnostics:services.diagnostics};
  };
  const host = await sdk.createAgentSessionRuntime(factory,{cwd:sessionManager.getCwd(),agentDir,sessionManager});
  await sdk.runRpcMode(host);
}
main().catch((error)=>{ process.stderr.write(`${error instanceof Error ? error.message : "Pi SDK 装配失败"}\n`); process.exitCode=1; });
