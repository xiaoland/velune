// Pi 1.0.2 owns this source's credential store and refresh lock. Only request
// auth crosses the helper boundary; refresh credentials never leave the SDK.
import { readFile, stat, realpath } from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createInterface } from "node:readline";

const SDK = "@earendil-works/pi-coding-agent";
const CAPABILITIES = Object.freeze({
  protocol: "responsesV1", endpoint: "https://api.openai.com/v1",
  explicitOutputCap: false, temperature: false, authentication: "subscription",
});

function validate(source) {
  if (source?.kind !== "harness" || source.harnessTypeId !== "pi" ||
      typeof source.providerId !== "string" || source.providerId !== "openai") {
    throw new Error("unsupported_source");
  }
  for (const key of ["authPath", "nodeBinary"]) {
    const value = source.settings?.[key];
    if (typeof value !== "string" || !isAbsolute(value) || value.includes("\0")) {
      throw new Error("invalid_source_path");
    }
  }
}

export async function createSourceRuntime(source, { resourcesDirectory = dirname(fileURLToPath(import.meta.url)), login = false } = {}) {
  validate(source);
  try {
    if (!(await stat(source.settings.authPath)).isFile()) throw new Error("invalid_auth_file");
  } catch (error) { if (!login || error.code !== "ENOENT") throw error; }
  const packageDirectory = join(resourcesDirectory, "node_modules", SDK);
  const manifest = JSON.parse(await readFile(join(packageDirectory, "package.json"), "utf8"));
  const entry = join(packageDirectory, manifest.exports["."].import);
  if (manifest.version !== "1.0.2") throw new Error("unsupported_sdk_version");
    const { ModelRuntime } = await import(pathToFileURL(entry).href);
  // AuthStorage is a public class in the pinned SDK's dist declaration, but is
  // not re-exported by the package root. This adapter intentionally binds to
  // that file contract to retain Pi's original refresh/persistence lock.
  const { AuthStorage } = await import(pathToFileURL(join(packageDirectory, "dist/core/auth-storage.js")).href);
  const store = AuthStorage.create(source.settings.authPath);
  // Missing, removed, or wrong-type credentials must fail before the SDK can
  // consult ambient auth. modify keeps the SDK's original cross-process lock.
  const requireOAuth = (value) => {
    if (value?.type !== "oauth") throw new Error("source_oauth_missing");
    return value;
  };
  const credentials = {
    read: async (id, options) => {
      if (id !== source.providerId) throw new Error("source_provider_mismatch");
      return requireOAuth(await store.read(id, options));
    },
    modify: (id, operation, options) => {
      if (id !== source.providerId) throw new Error("source_provider_mismatch");
      return store.modify(id, async (current) => operation(login ? current : requireOAuth(current)), options);
    },
    list: async (options) => (await store.list(options)).filter((item) => item.providerId === source.providerId),
    delete: async () => { throw new Error("source_logout_not_supported"); },
  };
  const runtime = await ModelRuntime.create({
    credentials, modelsPath: null, refreshOnCreate: false, allowModelNetwork: false,
  });
  return { runtime, store };
}

export async function resolveSource(source, { factory = createSourceRuntime, signal } = {}) {
  validate(source);
  const { runtime } = await factory(source);
  const result = await runtime.getAuth(source.providerId, { signal, env: {}, minOAuthValidityMs: 300_000 });
  const bearer = result?.auth?.apiKey;
  if (typeof bearer !== "string" || !bearer || result.auth.baseUrl ||
      (result.auth.headers && Object.keys(result.auth.headers).length)) {
    throw new Error("unsupported_request_auth");
  }
  return { contractVersion: 1, bearer, capabilities: CAPABILITIES };
}

export async function inspectSource(source, { factory = createSourceRuntime } = {}) {
  validate(source);
  const { store, runtime } = await factory(source);
  const entries = (await store.list()).filter((item) => item.providerId === source.providerId);
  const provider = runtime.getProvider(source.providerId);
  return { contractVersion: 1, providerId: source.providerId,
    configured: entries.some((item) => item.type === "oauth"), capabilities: CAPABILITIES,
    actions: provider?.auth?.oauth ? [{ id: "login", label: provider.auth.oauth.loginLabel ?? "登录" }] : [],
  };
}

export async function loginSource(source, interaction, { factory = createSourceRuntime, getDeviceId } = {}) {
  validate(source);
  // User-triggered login may create the first OAuth entry in the source. The
  // SDK still persists it through AuthStorage.modify under the original lock.
  const { runtime } = await factory(source, { login: true });
  await runtime.login(source.providerId, "oauth", interaction, { getDeviceId });
  return { contractVersion: 1, providerId: source.providerId, completed: true };
}

async function main() {
  const args = process.argv.slice(2);
  const value = (key) => { const index = args.indexOf(key); return index >= 0 ? args[index + 1] : undefined; };
  const source = JSON.parse(value("--source-json"));
  const operation = value("--operation");
  const output = (value) => process.stdout.write(JSON.stringify(value) + "\n");
  if (operation === "resolve") output(await resolveSource(source));
  else if (operation === "inspect") output(await inspectSource(source));
  else if (operation === "login") {
    const controller = new AbortController();
    const input = createInterface({ input: process.stdin });
    const pending = new Map(); let sequence = 0;
    input.on("line", (line) => {
      try {
        const value = JSON.parse(line);
        if (value.type === "cancel") controller.abort();
        else if (value.type === "answer" && typeof value.value === "string") pending.get(value.id)?.resolve(value.value);
      } catch { controller.abort(); }
    });
    input.on("close", () => controller.abort());
    try {
      const result = await loginSource(source, {
        signal: controller.signal,
        notify: (event) => {
          const { type: kind, message: text, ...fields } = event;
          output({ type: "notify", notification: { kind, ...(text ? { text } : {}), ...fields } });
        },
        prompt: (prompt) => new Promise((resolve, reject) => {
          const id = String(++sequence);
          const signal = prompt.signal ? AbortSignal.any([prompt.signal, controller.signal]) : controller.signal;
          const abort = () => {
            pending.delete(id);
            output({ type: "notify", notification: { kind: "prompt_cancelled", id } });
            reject(new Error("login_cancelled"));
          };
          if (signal.aborted) { abort(); return; }
          const finish = (value) => { signal.removeEventListener("abort", abort); pending.delete(id); resolve(value); };
          pending.set(id, { resolve: finish }); signal.addEventListener("abort", abort, { once: true });
          const { signal: _, type: kind, message: text, ...fields } = prompt;
          output({ type: "prompt", id, prompt: { kind, text, ...fields } });
        }),
      }, { getDeviceId: () => value("--device-id") });
      output({ type: "result", ok: true, ...result });
    } finally { input.close(); }
  } else throw new Error("unsupported_operation");
}

if (process.argv[1] && import.meta.url === pathToFileURL(await realpath(process.argv[1])).href) {
  main().catch(() => {
    // SDK error causes can contain upstream response bodies or credential data.
    if (process.argv.includes("login")) {
      process.stdout.write(JSON.stringify({ type: "result", ok: false,
        error: "Pi 认证来源操作失败，请检查来源配置后重试。", errorCode: "authentication_failed" }) + "\n");
    }
    process.stderr.write("Pi 认证来源操作失败\n");
    process.exitCode = 1;
  });
}
