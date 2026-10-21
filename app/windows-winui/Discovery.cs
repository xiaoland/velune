using Velune.Bindings;

namespace Velune;

// Finds installed commands the way `where` would, without running any launcher.
//
// npm and pnpm install three launchers per command: `x.cmd`, `x.ps1` and an
// extensionless `sh` script. Pi's installation resolver reads only the `sh`
// script's literal target, so Pi uses that one; Codex and Claude Code start
// their binary directly, so they use a native `.exe`, else the `.cmd` launcher.
static class Discovery
{
    public static BindingRuntimeDiscoveryCandidate[] Discover(VeluneApplication application)
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        if (string.IsNullOrEmpty(home))
        {
            throw new InvalidOperationException("找不到用户目录");
        }
        var node = NodeBinary() ?? throw new InvalidOperationException("没有找到 Node（node.exe），请先安装 Node 或手动添加运行时");
        // Where the agents keep their state, and where vendors' apps install them.
        var overrides = new Dictionary<string, string>();
        foreach (var key in new[] { "PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR", "DSH_HOME", "LOCALAPPDATA", "APPDATA" })
        {
            if (Environment.GetEnvironmentVariable(key) is { } value)
            {
                overrides[key] = value;
            }
        }
        var probes = new List<BindingRuntimeDiscoveryProbe>();
        foreach (var hint in application.RuntimeDiscoveryHints(home, overrides))
        {
            // PATH first, then where the vendor's own installer, app or editor extension put it.
            var binaries = CommandCandidates(hint.FamilyId, hint.Command);
            binaries.AddRange(hint.BundledBinaries.Where(bundled => !binaries.Contains(bundled)));
            probes.AddRange(binaries.Select(binary => new BindingRuntimeDiscoveryProbe(hint.FamilyId, binary, node, hint.AgentDirectory)));
        }
        return application.DiscoverRuntimes([.. probes]);
    }

    static List<string> SearchPath()
    {
        var directories = (Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator).ToList();
        // Package managers' global directories, for an app started before PATH was updated.
        foreach (var (variable, relative) in new[] { ("LOCALAPPDATA", "pnpm"), ("APPDATA", "npm"), ("LOCALAPPDATA", @"Programs\OpenAI\Codex\bin") })
        {
            if (Environment.GetEnvironmentVariable(variable) is { Length: > 0 } root)
            {
                directories.Add(Path.Combine(root, relative));
            }
        }
        return [.. directories.Where(Path.IsPathFullyQualified).Distinct()];
    }

    static bool IsShLauncher(string path)
    {
        try
        {
            using var file = File.OpenRead(path);
            var start = new byte[9];
            return file.Read(start) == start.Length && System.Text.Encoding.ASCII.GetString(start) == "#!/bin/sh";
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
    }

    // CommandCandidates are every installation of command a runtime family
    // can be probed with, in PATH order.
    static List<string> CommandCandidates(string family, string command)
    {
        var found = new List<string>();
        foreach (var directory in SearchPath())
        {
            var chosen = family == "pi"
                ? new[] { Path.Combine(directory, command) }.FirstOrDefault(launcher => File.Exists(launcher) && IsShLauncher(launcher))
                : new[] { command + ".exe", command + ".cmd" }.Select(name => Path.Combine(directory, name)).FirstOrDefault(File.Exists);
            if (chosen != null && !found.Contains(chosen))
            {
                found.Add(chosen);
            }
        }
        return found;
    }

    // NodeBinary is the first node.exe on PATH.
    static string? NodeBinary() => SearchPath().Select(directory => Path.Combine(directory, "node.exe")).FirstOrDefault(File.Exists);
}
