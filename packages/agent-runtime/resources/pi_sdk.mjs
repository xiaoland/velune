// Every Pi operation binds to the explicitly selected external installation.
import { readFileSync, realpathSync, openSync, readSync, closeSync } from "node:fs";
import { dirname, join, resolve, isAbsolute } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const adapters = {
  "pi-1.0.2": {packageName:"@earendil-works/pi-coding-agent",version:/^1\.0\.2$/,aiPackage:"@earendil-works/pi-ai"},
};

const packageNames = new Set(["@earendil-works/pi-coding-agent", "@badlogic/pi-coding-agent"]);

function owningPackage(cli) {
  let directory = dirname(cli);
  for (;;) {
    try {
      const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
      if (packageNames.has(manifest.name)) {
        if (typeof manifest.bin?.pi !== "string" || realpathSync(resolve(directory, manifest.bin.pi)) !== cli) throw new Error("invalid_sdk_manifest");
        return {cli, packageDirectory:directory, manifest};
      }
    } catch (error) { if (error.code !== "ENOENT") throw new Error("invalid_sdk_manifest"); }
    const parent = dirname(directory);
    if (parent === directory) return undefined;
    directory = parent;
  }
}

// Only read a known npm/pnpm launcher's literal CLI target. Never execute or
// evaluate the shell wrapper, its NODE_PATH, or any other environment setting.
export function resolvePiInstallation(binary) {
  if (typeof binary !== "string" || !isAbsolute(binary) || binary.includes("\0")) throw new Error("invalid_sdk_source");
  let executable;
  try { executable = realpathSync(binary); } catch { throw new Error("sdk_entrypoint_missing"); }
  const direct = owningPackage(executable);
  if (direct) return direct;
  const descriptor = openSync(executable, "r");
  let header;
  try { const bytes = Buffer.alloc(32768); const count = readSync(descriptor, bytes, 0, bytes.length, 0); header = bytes.subarray(0,count).toString("utf8"); }
  finally { closeSync(descriptor); }
  if (!/^#![^\n]*\b(?:ba)?sh(?:\s|$)/.test(header)) throw new Error("sdk_source_not_found");
  // Windows node.exe branches do not determine the native Node CLI target.
  const branches = header.split("\n").filter(line => /^\s*exec (?:"\$basedir\/node"|node) /.test(line));
  const targets = branches.map(line => {
    const match = /^\s*exec\s+(?:"\$basedir\/node"|node)\s+"([^"\n]+)"\s+"\$@"\s*$/.exec(line);
    if (!match) throw new Error("sdk_launcher_unsupported");
    const target = match[1].replace(/^\$basedir\//, dirname(executable) + "/");
    if (!isAbsolute(target) || /[$`\0]/.test(target)) throw new Error("sdk_launcher_unsupported");
    return resolve(target);
  });
  if (!targets.length || targets.some(target => target !== targets[0])) throw new Error("sdk_launcher_unsupported");
  let cli;
  try { cli = realpathSync(targets[0]); } catch { throw new Error("sdk_entrypoint_missing"); }
  const installation = owningPackage(cli);
  if (!installation) throw new Error("sdk_source_not_found");
  return installation;
}

export function resolvePiSdk({binary, runtimeTypeId, sdkVersion}) {
  const adapter = adapters[runtimeTypeId];
  if (!adapter) throw new Error("invalid_sdk_source");
  const {cli, packageDirectory:directory, manifest} = resolvePiInstallation(binary);
  if (manifest.name !== adapter.packageName) throw new Error("sdk_source_version_mismatch");
  const entry = manifest.exports?.["."]?.import;
  if (!adapter.version.test(manifest.version) || (sdkVersion && manifest.version !== sdkVersion)
    || typeof entry !== "string") throw new Error("sdk_source_version_mismatch");
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
        catch (error) { if (error.code === "ENOENT") continue; throw new Error("invalid_ai_sdk_manifest"); }
        const aiEntry = dependency.exports?.["."]?.import;
        if (dependency.name !== adapter.aiPackage || typeof aiEntry !== "string") throw new Error("invalid_ai_sdk_manifest");
        return import(pathToFileURL(resolve(path,aiEntry)).href);
      }
      throw new Error("ai_sdk_source_not_found");
    },
  };
}
