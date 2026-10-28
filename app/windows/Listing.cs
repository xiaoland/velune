namespace Velune;

// What a device's composer chooses among, from its latest listing.
sealed record Configuration(
    // The enabled runtime instances.
    IReadOnlyList<RuntimeInstance> Runtimes,
    IReadOnlyList<RuntimeTypeDescriptor> Types,
    // The working directories of the latest conversations, newest first.
    IReadOnlyList<string> Projects)
{
    public static readonly Configuration Empty = new([], [], []);

    // How the device's settings say conversations show.
    public OutlinePresentation Outline { get; init; } = OutlinePresentation.Popover;
    public LinkedConversationOpening LinkedOpening { get; init; } = LinkedConversationOpening.CurrentTab;
    public UsageDisplay Usage { get; init; } = UsageDisplay.Used;
    public UsageDisplay ContextUsage { get; init; } = UsageDisplay.Used;
    public IReadOnlyList<StatusLineItem> StatusLine { get; init; } = [StatusLineItem.ContextUsage];

    public RuntimeInstance? Runtime(string id) => Runtimes.FirstOrDefault(runtime => runtime.Id == id);

    public RuntimeTypeDescriptor? RuntimeType(string runtimeId) =>
        Runtime(runtimeId) is { } runtime ? Types.FirstOrDefault(type => type.Id == runtime.TypeId) : null;
}

// A device's conversation list as last read.
sealed class Listing
{
    public static readonly Listing Loading = new() { Loaded = false };

    // The shown conversations: not archived and not a sub-agent's.
    public List<ListedConversation> Conversations { get; private init; } = [];
    // The list's groups in order, and how the device's list is browsed.
    public ConversationGroup[] Groups { get; set; } = [];
    public ConversationBrowserPreferences? Preferences { get; set; }
    // Every project of the listed conversations, for moving and editing.
    public ConversationProject[] Projects { get; private init; } = [];
    // The list shows archived conversations instead of the others.
    public bool ShowsArchived { get; private init; }
    public Configuration Configuration { get; private init; } = Configuration.Empty;
    // The running or waiting conversations by ID.
    public Dictionary<string, ActiveConversation> Active { get; private init; } = [];
    public bool Loaded { get; private init; } = true;
    // ReadFailed tells the list could not be read; 问题 tells why.
    public bool ReadFailed { get; private init; }

    public static Listing From(ConfigurationSnapshot list, ActiveConversation[] active)
    {
        var projects = new List<string>();
        foreach (var listed in list.Conversations.OrderByDescending(listed => listed.Conversation.UpdatedAtUnixMs ?? 0))
        {
            if (listed.Project is { } key && list.ConversationProjects.FirstOrDefault(project => project.Path == key)?.PrimaryFolder is { } project && !projects.Contains(project))
            {
                projects.Add(project);
            }
        }
        return new Listing
        {
            Conversations = [.. list.Conversations.Where(listed => listed.Archived == list.ShowsArchived && listed.Conversation.ParentConversationId == null)],
            Projects = list.ConversationProjects,
            ShowsArchived = list.ShowsArchived,
            Groups = list.ConversationGroups,
            Preferences = list.ConversationBrowserPreferences,
            Configuration = new Configuration([.. list.RuntimeInstances.Where(runtime => runtime.Enabled)], list.RuntimeTypes, projects)
            {
                Outline = list.OutlinePresentation,
                LinkedOpening = list.LinkedConversationOpening,
                Usage = list.UsageDisplay,
                ContextUsage = list.ContextUsageDisplay,
                StatusLine = list.StatusLine,
            },
            Active = active.ToDictionary(conversation => conversation.ConversationId),
        };
    }

    // A failed read keeps what was shown, with the failure.
    public Listing Failed() => new()
    {
        Conversations = Conversations,
        Groups = Groups,
        Preferences = Preferences,
        Projects = Projects,
        ShowsArchived = ShowsArchived,
        Configuration = Configuration,
        Active = Active,
        ReadFailed = true,
    };

    // GroupID names a group of the list, as the key of its section.
    public static string GroupID(ConversationGroupKey? key) => key switch
    {
        ConversationGroupKey.Pinned => "pinned",
        ConversationGroupKey.Runtime runtime => "runtime:" + runtime.RuntimeId,
        ConversationGroupKey.Project project => "project:" + project.Path,
        _ => "all",
    };

    public string GroupTitle(ConversationGroup group) => group.Key switch
    {
        ConversationGroupKey.Pinned => "置顶",
        ConversationGroupKey.Runtime runtime => Configuration.Runtime(runtime.RuntimeId)?.Name ?? "Agent 运行时",
        ConversationGroupKey.Project when group.Name != null => group.Name,
        ConversationGroupKey.Project { Path: { } path } => LastPart(path),
        ConversationGroupKey.Project => "无项目",
        _ => "会话",
    };

    public static string LastPart(string path)
    {
        var trimmed = path.Replace('\\', '/').TrimEnd('/');
        var slash = trimmed.LastIndexOf('/');
        return slash < 0 ? trimmed : trimmed[(slash + 1)..];
    }
}
