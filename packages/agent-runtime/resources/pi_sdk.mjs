// Every Pi operation binds to the explicitly selected external installation.
import { readFileSync, realpathSync, openSync, readSync, closeSync } from "node:fs";
import { dirname, join, resolve, isAbsolute } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const adapters = {
  "pi-1.0.2": {packageName:"@earendil-works/pi-coding-agent",version:/^1\.0\.2$/,aiPackage:"@earendil-works/pi-ai"},
};

const packageNames = new Set(["@earendil-works/pi-coding-agent", "@badlogic/pi-coding-agent"]);
function fail(code, context, cause) {
  const detail = cause ? `${context}\nCaused by: ${cause.message ?? String(cause)}` : context;
  const error = new Error(detail, {cause});
  error.code = code;
  throw error;
}

function owningPackage(cli) {
  let directory = dirname(cli);
  for (;;) {
    const manifestPath = join(directory, "package.json");
    let text;
    try { text = readFileSync(manifestPath, "utf8"); }
    catch (error) {
      if (error.code !== "ENOENT") fail("invalid_sdk_manifest", `manifest=${manifestPath}`, error);
    }
    if (text !== undefined) {
      let manifest;
      try { manifest = JSON.parse(text); }
      catch (error) { fail("invalid_sdk_manifest", `manifest=${manifestPath}`, error); }
      if (packageNames.has(manifest.name)) {
        if (typeof manifest.bin?.pi !== "string") fail("invalid_sdk_manifest", `manifest=${manifestPath}; missing bin.pi`);
        const target = resolve(directory, manifest.bin.pi);
        let canonical;
        try { canonical = realpathSync(target); }
        catch (error) { fail("invalid_sdk_manifest", `manifest=${manifestPath}; binTarget=${target}`, error); }
        if (canonical !== cli) fail("invalid_sdk_manifest", `manifest=${manifestPath}; binTarget=${canonical}; expectedCli=${cli}`);
        return {cli, packageDirectory:directory, manifest};
      }
    }
    const parent = dirname(directory);
    if (parent === directory) return undefined;
    directory = parent;
  }
}

// Only read a known npm/pnpm launcher's literal CLI target. Never execute or
// evaluate the shell wrapper, its NODE_PATH, or any other environment setting.
export function resolvePiInstallation(binary) {
  try { return resolveInstallation(binary); }
  catch (error) { fail(error.code ?? "sdk_resolution_failed", `configuredBinary=${binary}`, error); }
}

function resolveInstallation(binary) {
  if (typeof binary !== "string" || !isAbsolute(binary) || binary.includes("\0")) fail("invalid_sdk_source", `invalid configured binary: ${binary}`);
  let executable;
  try { executable = realpathSync(binary); } catch (error) { fail("sdk_entrypoint_missing", `configuredBinary=${binary}; phase=entrypoint_realpath`, error); }
  const direct = owningPackage(executable);
  if (direct) return direct;
  const descriptor = openSync(executable, "r");
  let header;
  try { const bytes = Buffer.alloc(32768); const count = readSync(descriptor, bytes, 0, bytes.length, 0); header = bytes.subarray(0,count).toString("utf8"); }
  finally { closeSync(descriptor); }
  if (!/^#![^\n]*\b(?:ba)?sh(?:\s|$)/.test(header)) fail("sdk_source_not_found", `launcher header unsupported: ${binary}`);
  // Windows node.exe branches do not determine the native Node CLI target.
  const branches = header.split("\n").filter(line => /^\s*exec (?:"\$basedir\/node"|node) /.test(line));
  const targets = branches.map(line => {
    const match = /^\s*exec\s+(?:"\$basedir\/node"|node)\s+"([^"\n]+)"\s+"\$@"\s*$/.exec(line);
    if (!match) fail("sdk_launcher_unsupported", `launcher branch cannot be parsed: ${line}`);
    const target = match[1].replace(/^\$basedir\//, dirname(executable) + "/");
    if (!isAbsolute(target) || /[$`\0]/.test(target)) fail("sdk_launcher_unsupported", `invalid parsed target: ${target}`);
    return resolve(target);
  });
  if (!targets.length || targets.some(target => target !== targets[0])) fail("sdk_launcher_unsupported", `ambiguous launcher targets: ${targets.join(",")}`);
  let cli;
  try { cli = realpathSync(targets[0]); } catch (error) { fail("sdk_entrypoint_missing", `launcher=${executable}; parsedTarget=${targets[0]}; phase=launcher_target_realpath`, error); }
  const installation = owningPackage(cli);
  if (!installation) fail("sdk_source_not_found", `package manifest not found for target ${cli}`);
  return installation;
}

export function resolvePiSdk({binary, runtimeTypeId, sdkVersion}) {
  const adapter = adapters[runtimeTypeId];
  if (!adapter) fail("invalid_sdk_source", `runtimeTypeId=${runtimeTypeId}; configuredBinary=${binary}`);
  const {cli, packageDirectory:directory, manifest} = resolvePiInstallation(binary);
  if (manifest.name !== adapter.packageName) fail("sdk_source_version_mismatch", `configuredBinary=${binary}; expectedPackage=${adapter.packageName}; actualPackage=${manifest.name}`);
  const entry = manifest.exports?.["."]?.import;
  if (!adapter.version.test(manifest.version) || (sdkVersion && manifest.version !== sdkVersion)
    || typeof entry !== "string") fail("sdk_source_version_mismatch", `configuredBinary=${binary}; actualVersion=${manifest.version}; supportedVersion=${adapter.version}; requiredSdkVersion=${sdkVersion ?? "unspecified"}; entry=${entry}`);
  return {
    cli, packageDirectory:directory, version:manifest.version,
    importSdk:() => import(pathToFileURL(resolve(directory,entry)).href),
    importModule:(path) => import(pathToFileURL(join(directory,path)).href),
    importAi:() => {
      // Use Node's dependency search roots from this installation, never from
      // the app's resource directory or a different configured runtime.
      const require = createRequire(join(directory,"package.json"));
      for (const root of require.resolve.paths(adapter.aiPackage) ?? []) {
        let dependency;
        const path = join(root,adapter.aiPackage);
        try { dependency = JSON.parse(readFileSync(join(path,"package.json"),"utf8")); }
        catch (error) { if (error.code === "ENOENT") continue; fail("invalid_ai_sdk_manifest", `dependency=${path}`, error); }
        const aiEntry = dependency.exports?.["."]?.import;
        if (dependency.name !== adapter.aiPackage || typeof aiEntry !== "string") fail("invalid_ai_sdk_manifest", `dependency=${path}; package=${dependency.name}; entry=${aiEntry}`);
        return import(pathToFileURL(resolve(path,aiEntry)).href);
      }
      fail("ai_sdk_source_not_found", `configuredBinary=${binary}; dependency=${adapter.aiPackage}; packageDirectory=${directory}`);
    },
  };
}
