// Fixed Pi SDK adapter: metadata never contains secrets; resolve uses the
// original store and validates the imported execution binding before access.
import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { resolveSource } from "./pi_auth.mjs";

// Failures contain only allowlisted codes and stages, never SDK messages or source values.
class SourceDiagnostic extends Error {
  constructor(code, stage) { super(code); this.code = code; this.stage = stage; }
}
let stage = "input";
const fail = (code, failureStage = stage) => { throw new SourceDiagnostic(code, failureStage); };

async function main() {
  const args = process.argv.slice(2);
  const arg = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
  let source;
  try { source = JSON.parse(arg("--source-json") ?? "{}"); } catch { fail("invalid_source", "input"); }
  const nodeVersion = process.versions.node.split(".").map(Number);
  if (nodeVersion[0] < 22 || (nodeVersion[0] === 22 && nodeVersion[1] < 19)) fail("unsupported_node", "node");
  const reject = () => fail("invalid_source");
  const emit = (value) => process.stdout.write(JSON.stringify(value) + "\n");
  if (source.kind !== "harness" || source.harnessTypeId !== "pi") reject();
  const settings = source.settings ?? {};
  const sourceDir = settings.sourceDir || settings.agentDir;
  const modelsPath = settings.modelsPath || join(sourceDir ?? "", "models.json");
  const authPath = settings.authPath || join(sourceDir ?? "", "auth.json");
  for (const [path, code] of [[sourceDir,"invalid_source_directory"],[modelsPath,"invalid_models_path"],[authPath,"invalid_auth_path"],[settings.nodeBinary,"invalid_node_path"]]) {
    if (typeof path !== "string" || !isAbsolute(path) || path.includes("\0")) fail(code,"input");
  }
  stage = "source_directory";
  let directory;
  try { directory = await stat(sourceDir); } catch (error) {
    if (error.code === "ENOENT") fail("source_directory_missing");
    if (["EACCES","EPERM"].includes(error.code)) fail("source_directory_permission");
    fail("source_directory_unavailable");
  }
  if (!directory.isDirectory()) fail("source_directory_not_directory");
  stage = "sdk";
  const packageDir = new URL("node_modules/@earendil-works/pi-coding-agent/", import.meta.url);
  const manifest = JSON.parse(await readFile(new URL("package.json", packageDir), "utf8"));
  if (manifest.version !== "1.0.2") fail("unsupported_sdk");
  const { ModelRuntime } = await import(new URL(manifest.exports["."].import, packageDir).href);
  const { getSupportedThinkingLevels } = await import("@earendil-works/pi-ai");
  const { ModelConfig } = await import(new URL("dist/core/model-config.js", packageDir).href);
  const { InMemoryCodingAgentModelsStore } = await import(new URL("dist/core/models-store.js", packageDir).href);
  const { AuthStorage, ReadOnlyAuthStorage, readStoredCredential } = await import(new URL("dist/core/auth-storage.js", packageDir).href);
  const { isCommandConfigValue, getConfigValueEnvVarNames, resolveConfigValue } = await import(new URL("dist/core/resolve-config-value.js", packageDir).href);
  stage = "models";
  const config = await ModelConfig.load(modelsPath);
  if (config.getError()) fail("invalid_models");
  stage = "auth";
  const credentials = new ReadOnlyAuthStorage(authPath);
  const stored = await credentials.list();
  stage = "model_runtime";
  const runtime = await ModelRuntime.create({ modelsPath, credentials,
    modelsStore: new InMemoryCodingAgentModelsStore(), refreshOnCreate: false, allowModelNetwork: false });
  if (runtime.getError()) fail("invalid_model_runtime");
  stage = "projection";
  const protocol = (api) => ({ "openai-completions": "chatCompletionsV1", "openai-responses": "responsesV1" }[api] ?? null);
  const safeEndpoint = (value) => {
    try { const url = new URL(value); return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash ? url.origin + url.pathname.replace(/\/+$/, "") : null; }
    catch { return null; }
  };
  const levels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
  const responsesDefaults = { supportsDeveloperRole: true, supportsMidConvoSystemMessages: false,
    supportsLongCacheRetention: true, supportsStrictMode: false, supportsOpenAIGrammarTools: false,
    supportsAdditionalTools: false, supportsToolSearch: false, supportsExplicitPromptCacheMode: false,
    supportsMaxOutputTokens: true };
  const compatibility = (raw, model) => model.compat ?? raw?.compat ?? {};
  const projection = (raw, model) => {
    const available = getSupportedThinkingLevels(model);
    const thinkingLevelMap = Object.fromEntries(levels.map((level) => [level,
      available.includes(level) ? (["none", ...levels].includes(model.thinkingLevelMap?.[level] ?? level) ? (model.thinkingLevelMap?.[level] ?? level) : null) : null]));
    const result = { reasoningEnabled: model.reasoning, thinkingLevelMap, responsesCompat: null };
    if (protocol(model.api) === "responsesV1") {
      const compat = compatibility(raw, model);
      result.responsesCompat = Object.fromEntries(Object.entries(responsesDefaults).map(([key, fallback]) => [key, typeof compat[key] === "boolean" ? compat[key] : fallback]));
    }
    return result;
  };
  const canonical = (value) => JSON.stringify(value, (_, entry) => entry && !Array.isArray(entry) && typeof entry === "object"
    ? Object.fromEntries(Object.entries(entry).sort(([a], [b]) => a.localeCompare(b))) : entry);
  const populated = (value) => value != null && (typeof value !== "object" || Object.keys(value).length > 0);
  const issues = (raw, model) => {
    const result = [];
    if (!protocol(model.api)) result.push("当前网关不支持此 Pi 协议");
    if (!safeEndpoint(model.baseUrl ?? raw?.baseUrl)) result.push("服务地址无效或包含不能展示的认证信息");
    if (populated(raw?.headers) || populated(model.headers)) result.push("尚不支持来源中的自定义请求头");
    if (raw?.authHeader === false) result.push("尚不支持来源中的认证请求头规则");
    const compat = compatibility(raw, model);
    if (protocol(model.api) === "responsesV1") {
      if (Object.entries(compat).some(([key, value]) => !(key in responsesDefaults) || typeof value !== "boolean")) result.push("尚不支持来源中的 Responses 兼容选项");
    } else if (populated(compat)) result.push("尚不支持来源中的 Chat Completions 兼容选项");
    if (populated(model.samplingParams) || populated(model.samplingParamsByThinkingLevel)) result.push("尚不支持来源中的自定义采样参数");
    if (Object.entries(model.thinkingLevelMap ?? {}).some(([level, mapped]) => !levels.includes(level) || (mapped != null &&
        (protocol(model.api) === "responsesV1" ? !["none", ...levels].includes(mapped) : mapped !== level)))) result.push("尚不支持来源中的推理级别转换");
    return result;
  };
  // Parse-only SDK functions guard resolution. Every referenced variable must
  // exist in this same auth record, so the SDK cannot fall back to process.env.
  const keyShape = (value, env = {}) => {
    if (typeof value !== "string" || !value) return { kind: "none", ready: false, reason: "尚未配置可用 API key" };
    if (isCommandConfigValue(value)) return { kind: "command", ready: false, reason: "命令凭据不会被执行" };
    const names = getConfigValueEnvVarNames(value);
    if (names.some((name) => typeof env[name] !== "string" || !env[name])) return { kind: "environment", ready: false, reason: "不会读取运行环境中的凭据变量" };
    return { kind: names.length ? "stored_environment" : "literal_api_key", ready: true, reason: null, variables: names };
  };
  const authShape = (raw, id, endpoint, protocolID) => {
    let shape;
    let location;
    if (raw?.apiKey !== undefined) { shape = keyShape(raw.apiKey); location = "models"; }
    else {
      const credential = readStoredCredential(id, authPath);
      location = "auth";
      if (credential?.type === "oauth") {
        const supported = id === "openai" && endpoint === "https://api.openai.com/v1" && protocolID === "responsesV1";
        shape = { kind: "oauth", ready: supported, reason: supported ? null : "尚不支持此来源的 OAuth 协议" };
      } else shape = keyShape(credential?.key, credential?.env ?? {});
    }
    const labels = { oauth: "Pi OAuth 认证", literal_api_key: "原来源中的 API key", stored_environment: "原认证记录内的凭据变量", command: "命令凭据（不会执行）", environment: "环境变量引用（需配置认证）", none: "尚未配置认证" };
    return { ...shape, location, statusLabel: labels[shape.kind] };
  };
  const providerId = source.providerId;
  const operation = arg("--operation") ?? "preview";
  if (operation === "resolve" || operation === "inspect") {
    stage = "credential_binding";
    if (typeof providerId !== "string" || !providerId) reject();
    const raw = config.getProvider(providerId);
    const ids = JSON.parse(settings.bindingModelIds ?? "[]");
    if (!Array.isArray(ids) || !ids.length || ids.some((id) => typeof id !== "string")) reject();
    const selected = ids.map((id) => runtime.getModel(providerId, id));
    if (selected.some((model) => !model || issues(raw, model).length)) reject();
    const endpoint = safeEndpoint(selected[0].baseUrl ?? raw?.baseUrl);
    const protocolID = protocol(selected[0].api);
    if (!endpoint || !protocolID || selected.some((model) => safeEndpoint(model.baseUrl ?? raw?.baseUrl) !== endpoint || protocol(model.api) !== protocolID)) reject();
    if (settings.bindingEndpoint !== endpoint || settings.bindingProtocol !== protocolID ||
        (arg("--endpoint") && arg("--endpoint") !== endpoint) || (arg("--protocol") && arg("--protocol") !== protocolID)) reject();
    const approvedProjection = JSON.parse(settings.bindingProjection ?? "{}");
    if (selected.some((model) => canonical(approvedProjection[model.id]) !== canonical(projection(raw, model)))) reject();
    const auth = authShape(raw, providerId, endpoint, protocolID);
    const subscription = auth.kind === "oauth";
    const capabilities = { protocol: protocolID, endpoint, explicitOutputCap: !subscription, temperature: !subscription };
    const loginSupported = providerId === "openai" && endpoint === "https://api.openai.com/v1" && protocolID === "responsesV1" && raw?.apiKey === undefined && !stored.some((entry) => entry.providerId === providerId && entry.type !== "oauth");
    if (operation === "inspect") {
      emit({ contractVersion: 1, providerId, configured: auth.ready, capabilities, actions: loginSupported ? [{ id: "login", label: "登录…" }] : [] });
      return;
    }
    if (!auth.ready) reject();
    if (settings.credentialLocation && settings.credentialLocation !== auth.location) reject();
    if (subscription) {
      const result = await resolveSource({ ...source, settings: { ...settings, authPath } });
      emit(result); return;
    }
    let bearer;
    if (auth.location === "models") {
      // This immutable ModelConfig snapshot was already validated above.
      bearer = resolveConfigValue(raw.apiKey, {});
    } else {
      await AuthStorage.create(authPath).modify(providerId, async (credential) => {
        if (credential?.type !== "api_key" || !keyShape(credential.key, credential.env ?? {}).ready) reject();
        bearer = resolveConfigValue(credential.key, credential.env ?? {});
        return undefined;
      });
    }
    if (typeof bearer !== "string" || !bearer) reject();
    emit({ contractVersion: 1, bearer, capabilities }); return;
  }
  if (operation !== "preview") reject();
  const configuredIds = new Set([...config.getProviderIds(), ...stored.map((entry) => entry.providerId)]);
  const providers = [];
  const warnings = [];
  for (const provider of runtime.getProviders()) {
    if (!configuredIds.has(provider.id)) continue;
    // These provider IDs are owned by Velune's injected execution adapter,
    // including catalogs left by older releases in PI_HOME.
    if (["velune-gateway", "velune"].includes(provider.id)) {
      warnings.push("已跳过 Velune 生成的网关目录；它不是上游 AI 提供商。");
      continue;
    }
    const raw = config.getProvider(provider.id);
    const groups = new Map();
    for (const model of runtime.getModels(provider.id)) {
      const modelProtocol = protocol(model.api);
      const endpoint = safeEndpoint(model.baseUrl ?? raw?.baseUrl);
      const key = `${endpoint ?? ""}|${modelProtocol ?? model.api}`;
      if (!groups.has(key)) groups.set(key, { endpoint: endpoint ?? "", protocol: modelProtocol, models: [] });
      const unsupportedFeatures = issues(raw, model);
      groups.get(key).models.push({ sourceProviderId: provider.id, id: model.id, name: model.name, api: model.api,
        protocol: modelProtocol, input: model.input, reasoning: model.reasoning,
        reasoningLevels: getSupportedThinkingLevels(model), thinkingLevelMap: null, piProjection: projection(raw, model),
        contextWindow: model.contextWindow, maxTokens: model.maxTokens,
        cost: model.cost, promptCache: model.promptCache ?? null,
        supported: unsupportedFeatures.length === 0, unsupportedReason: unsupportedFeatures[0] ?? null, unsupportedFeatures });
    }
    for (const group of groups.values()) providers.push({ sourceProviderId: provider.id, name: provider.name,
      endpoint: group.endpoint, protocol: group.protocol,
      auth: authShape(raw, provider.id, group.endpoint, group.protocol), models: group.models });
  }
  const snapshot = { contractVersion: 1, sdkVersion: manifest.version, sourceDir, modelsPath, authPath, providers, warnings };
  const sourceFingerprint = `pi_${createHash("sha256").update(JSON.stringify(snapshot)).digest("hex")}`;
  emit({ ...snapshot, sourceFingerprint });
}
main().catch((error) => {
  const diagnostic = error instanceof SourceDiagnostic ? {code:error.code,phase:error.stage} : {code:"adapter_failed",phase:stage};
  process.stderr.write(JSON.stringify({contractVersion:1,diagnostic}) + "\n");
  process.exitCode = 1;
});
