// Fixed Pi SDK assembly: source PI_HOME owns its settings, credentials and sessions;
// Velune appends its current gateway provider catalog for this execution.
import { readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
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
  const sourceModelsPath = join(agentDir,"models.json");
  const modelRuntime = await sdk.ModelRuntime.create({authPath:join(agentDir,"auth.json"),modelsPath:sourceModelsPath});
  const modelRuntimeError = modelRuntime.getError();
  if (modelRuntimeError) fail(`Pi 源模型目录无效（${sourceModelsPath}）：${modelRuntimeError}`);
  const gatewayCatalog = JSON.parse(readFileSync(modelsPath,"utf8"));
  for (const [providerId, provider] of Object.entries(gatewayCatalog.providers ?? {})) {
    modelRuntime.registerProvider(providerId, provider);
  }
  const sessionPath = value("--session"), sessionDir = value("--session-dir");
  const sessionManager = sessionPath ? sdk.SessionManager.open(sessionPath, sessionDir) : sdk.SessionManager.create(process.cwd(), sessionDir);
  const trustStore = new sdk.ProjectTrustStore(agentDir), trustByCwd = new Map();
  const factory = async ({cwd,agentDir,sessionManager,sessionStartEvent,projectTrustContext}) => {
    const hasProjectResources = sdk.hasTrustRequiringProjectResources(cwd);
    const cached = trustByCwd.get(cwd);
    const shouldResolve = cached === undefined && hasProjectResources;
    const projectTrusted = shouldResolve ? false : (cached ?? (!hasProjectResources || trustStore.get(cwd) === true));
    const settingsManager = sdk.SettingsManager.create(cwd,agentDir,{projectTrusted});
    const services = await sdk.createAgentSessionServices({cwd,agentDir,settingsManager,modelRuntime,
      resourceLoaderOptions:{additionalExtensionPaths:[extension]},
      resourceLoaderReloadOptions:shouldResolve ? {resolveProjectTrust:async ({extensionsResult})=> {
        const trusted = await resolveProjectTrusted({cwd,trustStore,defaultProjectTrust:settingsManager.getDefaultProjectTrust(),extensionsResult,
          projectTrustContext:projectTrustContext ?? createProjectTrustContext({cwd,mode:"rpc",settingsManager,hasUI:false})});
        trustByCwd.set(cwd,trusted); return trusted;
      }} : undefined,
    });
    const model = services.modelRuntime.getModel("velune","auto");
    if (!model) fail("Velune 网关模型未注册");
    const result = await sdk.createAgentSessionFromServices({services,sessionManager,model,thinkingLevel:"off",sessionStartEvent});
    return {...result,services,diagnostics:services.diagnostics};
  };
  const host = await sdk.createAgentSessionRuntime(factory,{cwd:sessionManager.getCwd(),agentDir,sessionManager});
  await sdk.runRpcMode(host);
}
main().catch((error)=>{ process.stderr.write(`${error instanceof Error ? error.message : "Pi SDK 装配失败"}\n`); process.exitCode=1; });
