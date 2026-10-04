// The core owns source adapter selection; the platform secret helper only
// invokes this sealed entry point and relays its result without interpreting it.
const args = process.argv.slice(2);
const index = args.indexOf("--source-json");
try {
  const source = JSON.parse(args[index + 1]);
  if (index < 0 || source.kind !== "harness" || source.harnessTypeId !== "pi") throw new Error("unsupported_source");
  if (source.settings?.bindingProtocol) await import("./pi_provider_import.mjs");
  else {
    const { resolveSource } = await import("./pi_auth.mjs");
    process.stdout.write(JSON.stringify(await resolveSource(source)) + "\n");
  }
} catch {
  process.stderr.write("认证来源解析失败。\n");
  process.exitCode = 1;
}
