using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Velune.Bindings;

namespace Velune;

enum SubagentModel { Parent, Native, Velune }

// One environment variable or header of an MCP server; its value may be a secret.
sealed class Pair
{
    public string Key = "";
    public string Value = "";
}

// A resource as the library's editor holds it: plain fields for every
// kind, so an edit outlives the window and the app can tell what is
// unsaved. The core's template is made from it only to save.
sealed class Draft
{
    const string SkillEntry = "SKILL.md";

    // The resource's ID; empty until it is saved.
    public string Id = "";
    public BindingRuntimeResourceKind Kind;
    public string Name = "";
    public bool Dirty;
    // 指令: the whole file. 技能: its SKILL.md.
    public string Body = "";
    // A skill's other files, kept as they are.
    public BindingTemplateSkillFile[] Files = [];
    public bool EntryExecutable;
    // MCP 服务器.
    public bool Http;
    public string Command = "";
    public string Args = "";
    public string Cwd = "";
    public string Url = "";
    public string Timeout = "";
    public List<Pair> Env = [];
    public List<Pair> Headers = [];
    // 子 Agent.
    public string Description = "";
    public string Instructions = "";
    public SubagentModel Model;
    public string NativeModel = "";
    public string VeluneModel = "";
    public string Effort = "";
    public string Tools = "";
    public List<string> Skills = [];

    public static Draft New(BindingRuntimeResourceKind kind) => new()
    {
        Kind = kind,
        Dirty = true,
        Name = kind == BindingRuntimeResourceKind.UserInstructions ? "指令" : "",
        Body = kind == BindingRuntimeResourceKind.Skills ? "---\nname: \ndescription: \n---\n\n" : "",
    };

    public static Draft From(BindingResourceTemplate template)
    {
        var draft = new Draft { Id = template.Id, Name = template.Name };
        switch (template.Content)
        {
            case BindingResourceTemplateContent.Instructions instructions:
                draft.Kind = BindingRuntimeResourceKind.UserInstructions;
                draft.Body = instructions.Body;
                break;
            case BindingResourceTemplateContent.Skill skill:
                draft.Kind = BindingRuntimeResourceKind.Skills;
                var entry = skill.Files.FirstOrDefault(file => file.Path == SkillEntry);
                draft.Body = entry != null ? Encoding.UTF8.GetString(entry.Bytes) : "";
                draft.EntryExecutable = entry?.Executable ?? false;
                draft.Files = [.. skill.Files.Where(file => file.Path != SkillEntry)];
                break;
            case BindingResourceTemplateContent.Mcp mcp:
                draft.Kind = BindingRuntimeResourceKind.McpServers;
                draft.SetTransport(mcp.Transport);
                draft.Timeout = mcp.TimeoutSeconds?.ToString() ?? "";
                break;
            case BindingResourceTemplateContent.Subagent agent:
                draft.Kind = BindingRuntimeResourceKind.NativeAgentDefinitions;
                draft.Description = agent.Description;
                draft.Instructions = agent.InstructionsValue;
                draft.Model = agent.VeluneModel != null ? SubagentModel.Velune : agent.Model != null ? SubagentModel.Native : SubagentModel.Parent;
                draft.NativeModel = agent.Model ?? "";
                draft.VeluneModel = agent.VeluneModel ?? "";
                draft.Effort = agent.Effort ?? "";
                draft.Tools = string.Join(", ", agent.Tools);
                draft.Skills = [.. agent.Skills];
                break;
        }
        return draft;
    }

    void SetTransport(BindingTemplateMcpTransport transport)
    {
        switch (transport)
        {
            case BindingTemplateMcpTransport.Stdio stdio:
                Http = false;
                Command = stdio.Command;
                Args = string.Join("\n", stdio.Args);
                Cwd = stdio.Cwd ?? "";
                Env = [.. stdio.Env.OrderBy(entry => entry.Key).Select(entry => new Pair { Key = entry.Key, Value = entry.Value })];
                break;
            case BindingTemplateMcpTransport.Http http:
                Http = true;
                Url = http.Url;
                Headers = [.. http.Headers.OrderBy(entry => entry.Key).Select(entry => new Pair { Key = entry.Key, Value = entry.Value })];
                break;
        }
    }

    // Copy is a new, unsaved resource with this one's content.
    public Draft Copy()
    {
        var copy = (Draft)MemberwiseClone();
        copy.Id = "";
        copy.Dirty = true;
        copy.Name = Kind == BindingRuntimeResourceKind.UserInstructions ? Name + " 副本" : Name + "-copy";
        copy.Env = [.. Env.Select(pair => new Pair { Key = pair.Key, Value = pair.Value })];
        copy.Headers = [.. Headers.Select(pair => new Pair { Key = pair.Key, Value = pair.Value })];
        copy.Skills = [.. Skills];
        return copy;
    }

    // Template is the resource to save; a field that does not read throws
    // FormatException with what is wrong.
    public BindingResourceTemplate Template() => new(Id, Name.Trim(), Kind switch
    {
        BindingRuntimeResourceKind.UserInstructions => new BindingResourceTemplateContent.Instructions(Body),
        BindingRuntimeResourceKind.Skills => new BindingResourceTemplateContent.Skill(
            [new BindingTemplateSkillFile(SkillEntry, Encoding.UTF8.GetBytes(Body), EntryExecutable), .. Files]),
        BindingRuntimeResourceKind.McpServers => new BindingResourceTemplateContent.Mcp(Transport(), TimeoutSeconds()),
        _ => new BindingResourceTemplateContent.Subagent(
            Description,
            Instructions,
            Model == SubagentModel.Native ? Required(NativeModel, "原生模型 ID") : null,
            Model == SubagentModel.Velune ? Required(VeluneModel, "Velune 模型") : null,
            [.. Tools.Split([',', '，', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)],
            Effort.Length > 0 ? Effort : null,
            [.. Skills]),
    });

    static string Required(string value, string what) =>
        value.Trim() is { Length: > 0 } given ? given : throw new FormatException($"请填写{what}。");

    BindingTemplateMcpTransport Transport() => Http
        ? new BindingTemplateMcpTransport.Http(Required(Url, "URL"), Map(Headers, "请求头"))
        : new BindingTemplateMcpTransport.Stdio(
            Required(Command, "命令"),
            [.. Args.Split('\n').Select(arg => arg.TrimEnd('\r')).Where(arg => arg.Length > 0)],
            Map(Env, "环境变量"),
            Cwd.Trim().Length > 0 ? Cwd.Trim() : null);

    static Dictionary<string, string> Map(List<Pair> pairs, string what)
    {
        var map = new Dictionary<string, string>();
        foreach (var pair in pairs.Where(pair => pair.Key.Trim().Length > 0 || pair.Value.Length > 0))
        {
            var key = pair.Key.Trim();
            if (key.Length == 0)
            {
                throw new FormatException($"{what}的名称不能为空。");
            }
            if (!map.TryAdd(key, pair.Value))
            {
                throw new FormatException($"{what}“{key}”重复。");
            }
        }
        return map;
    }

    uint? TimeoutSeconds() => Timeout.Trim() switch
    {
        "" => null,
        var text when uint.TryParse(text, out var seconds) && seconds > 0 => seconds,
        _ => throw new FormatException("超时须为正整数秒。"),
    };

    // McpJson is the server's declaration as JSON, the shape MCP
    // documentation shows, to read or paste over.
    public string McpJson()
    {
        var server = new JsonObject();
        if (Http)
        {
            server["type"] = "http";
            server["url"] = Url;
            server["headers"] = Object(Headers);
        }
        else
        {
            server["command"] = Command;
            server["args"] = new JsonArray([.. Args.Split('\n').Select(arg => arg.TrimEnd('\r')).Where(arg => arg.Length > 0).Select(arg => (JsonNode?)JsonValue.Create(arg))]);
            server["env"] = Object(Env);
            if (Cwd.Trim().Length > 0)
            {
                server["cwd"] = Cwd.Trim();
            }
        }
        return server.ToJsonString(new JsonSerializerOptions { WriteIndented = true, Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });

        static JsonObject Object(List<Pair> pairs)
        {
            var map = new JsonObject();
            foreach (var pair in pairs.Where(pair => pair.Key.Trim().Length > 0))
            {
                map[pair.Key.Trim()] = pair.Value;
            }
            return map;
        }
    }

    // ReadMcpJson takes a pasted declaration: one server's object, or
    // {"mcpServers": {name: {...}}} or {name: {...}} with a single server,
    // whose name fills an empty one. It returns what is wrong, or the
    // fields it left out.
    public (string? Error, string[] Ignored) ReadMcpJson(string text)
    {
        JsonObject? server;
        string? name = null;
        try
        {
            server = JsonNode.Parse(text) as JsonObject;
        }
        catch (JsonException failure)
        {
            return ("JSON 无法解析：" + failure.Message, []);
        }
        if (server?["mcpServers"] is JsonObject servers)
        {
            server = servers;
        }
        if (server != null && !server.ContainsKey("command") && !server.ContainsKey("url") && server.Count == 1 && server.First().Value is JsonObject only)
        {
            name = server.First().Key;
            server = only;
        }
        if (server == null)
        {
            return ("需要一个 MCP 服务器的 JSON 对象。", []);
        }
        try
        {
            var ignored = new List<string>();
            if (server["url"] is JsonNode url)
            {
                Http = true;
                Url = url.GetValue<string>();
                Headers = Pairs(server["headers"]);
                ignored.AddRange(server.Select(entry => entry.Key).Where(key => key is not ("url" or "headers" or "type")));
            }
            else if (server["command"] is JsonNode command)
            {
                Http = false;
                Command = command.GetValue<string>();
                Args = string.Join("\n", (server["args"] as JsonArray ?? []).Select(arg => arg!.GetValue<string>()));
                Env = Pairs(server["env"]);
                Cwd = server["cwd"]?.GetValue<string>() ?? "";
                ignored.AddRange(server.Select(entry => entry.Key).Where(key => key is not ("command" or "args" or "env" or "cwd" or "type")));
            }
            else
            {
                return ("缺少 command 或 url。", []);
            }
            if (name != null && Name.Trim().Length == 0)
            {
                Name = name;
            }
            return (null, [.. ignored]);
        }
        catch (Exception failure) when (failure is InvalidOperationException or FormatException)
        {
            return ("字段的类型不对：" + failure.Message, []);
        }

        static List<Pair> Pairs(JsonNode? node) =>
            node is JsonObject map ? [.. map.Select(entry => new Pair { Key = entry.Key, Value = entry.Value?.GetValue<string>() ?? "" })] : [];
    }
}

// The library's drafts while the app runs: closing the window keeps them,
// and quitting asks about the unsaved ones.
static class Library
{
    // By resource ID, or "new N" for one not saved yet.
    public static readonly Dictionary<string, Draft> Drafts = [];
    static int created;

    public static string Add(Draft draft)
    {
        var key = draft.Id.Length > 0 ? draft.Id : "new " + ++created;
        Drafts[key] = draft;
        return key;
    }

    public static IReadOnlyList<string> Unsaved => [.. Drafts.Values.Where(draft => draft.Dirty).Select(draft => draft.Name.Trim() is { Length: > 0 } name ? name : "未命名")];
}
