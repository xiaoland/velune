using Velune.Bindings;

namespace Velune;

// What a device's composer chooses among, from its latest listing.
sealed record Configuration(
    // The enabled runtime instances.
    IReadOnlyList<BindingRuntimeInstance> Runtimes,
    IReadOnlyList<BindingRuntimeTypeDescriptor> Types,
    // The working directories of the latest conversations, newest first.
    IReadOnlyList<string> Projects)
{
    public static readonly Configuration Empty = new([], [], []);

    // How the device's settings say conversations show.
    public BindingOutlinePresentation Outline { get; init; } = BindingOutlinePresentation.Popover;
    public BindingLinkedConversationOpening LinkedOpening { get; init; } = BindingLinkedConversationOpening.CurrentTab;
    public BindingUsageDisplay Usage { get; init; } = BindingUsageDisplay.Used;
    public IReadOnlyList<BindingStatusLineItem> StatusLine { get; init; } = [BindingStatusLineItem.ContextUsage];

    public BindingRuntimeInstance? Runtime(string id) => Runtimes.FirstOrDefault(runtime => runtime.Id == id);

    public BindingRuntimeTypeDescriptor? RuntimeType(string runtimeId) =>
        Runtime(runtimeId) is { } runtime ? Types.FirstOrDefault(type => type.Id == runtime.TypeId) : null;
}

// A device's conversation list as last read.
sealed class Listing
{
    public static readonly Listing Loading = new() { Loaded = false };

    // The shown conversations: not archived and not a sub-agent's.
    public List<BindingListedConversation> Conversations { get; private init; } = [];
    // The list's groups in order, and how the device's list is browsed.
    public BindingConversationGroup[] Groups { get; set; } = [];
    public BindingConversationBrowserPreferences? Preferences { get; set; }
    // Every project of the listed conversations, for moving and editing.
    public BindingConversationProject[] Projects { get; private init; } = [];
    // The list shows archived conversations instead of the others.
    public bool ShowsArchived { get; private init; }
    public Configuration Configuration { get; private init; } = Configuration.Empty;
    // The running or waiting conversations by ID.
    public Dictionary<string, BindingActiveConversation> Active { get; private init; } = [];
    public bool Loaded { get; private init; } = true;
    public string Failure { get; set; } = "";

    public static Listing From(BindingConfigurationSnapshot list, BindingActiveConversation[] active)
    {
        var projects = new List<string>();
        foreach (var listed in list.Conversations.OrderByDescending(listed => listed.Conversation.UpdatedAtUnixMs ?? 0))
        {
            if (listed.Project is { } project && !projects.Contains(project))
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
                StatusLine = list.StatusLine,
            },
            Active = active.ToDictionary(conversation => conversation.ConversationId),
        };
    }

    // A failed read keeps what was shown, with the failure.
    public Listing Failed(string failure) => new()
    {
        Conversations = Conversations,
        Groups = Groups,
        Preferences = Preferences,
        Projects = Projects,
        ShowsArchived = ShowsArchived,
        Configuration = Configuration,
        Active = Active,
        Failure = failure,
    };

    // GroupID names a group of the list, as the key of its section.
    public static string GroupID(BindingConversationGroupKey? key) => key switch
    {
        BindingConversationGroupKey.Pinned => "pinned",
        BindingConversationGroupKey.Runtime runtime => "runtime:" + runtime.RuntimeId,
        BindingConversationGroupKey.Project project => "project:" + project.Path,
        _ => "all",
    };

    public string GroupTitle(BindingConversationGroup group) => group.Key switch
    {
        BindingConversationGroupKey.Pinned => "置顶",
        BindingConversationGroupKey.Runtime runtime => Configuration.Runtime(runtime.RuntimeId)?.Name ?? "Agent 运行时",
        BindingConversationGroupKey.Project when group.Name != null => group.Name,
        BindingConversationGroupKey.Project { Path: { } path } => LastPart(path),
        BindingConversationGroupKey.Project => "无项目",
        _ => "会话",
    };

    public static string LastPart(string path)
    {
        var trimmed = path.Replace('\\', '/').TrimEnd('/');
        var slash = trimmed.LastIndexOf('/');
        return slash < 0 ? trimmed : trimmed[(slash + 1)..];
    }
}
