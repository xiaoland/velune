using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// The window's keyboard: the command palette, 打开会话, the next turn's
// choices, 转到待处理的会话 and the shortcut sheet.
public sealed partial class WorkspaceWindow
{
    CommandPalette? palette;

    static readonly (SettingsPage Page, string Name, string English)[] settingsPages =
    [
        (SettingsPage.General, "通用", "General"),
        (SettingsPage.Providers, "AI提供商", "AI Providers"),
        (SettingsPage.Routes, "模型路由", "Model Routes"),
        (SettingsPage.Runtimes, "Agent 运行时", "Agent Runtimes"),
        (SettingsPage.Devices, "设备", "Devices"),
    ];

    internal void RunCommand(Command command) => Run(command);

    // CanRun tells whether a command does something now; the palette and
    // the composer's / list both ask.
    internal bool CanRun(Command command)
    {
        var view = FocusedView;
        var model = view?.Model;
        var composing = view?.Composing == true;
        return command switch
        {
            Command.Attach => composing && model!.Busy == false,
            Command.ChooseRuntime => composing && model!.Configuration.Runtimes.Count > 0,
            Command.ChooseModel => composing && model!.Next.Length > 0,
            Command.ChooseEffort => composing && model!.NextTurn.Efforts.Length > 0,
            Command.ChoosePermission => composing && model!.NextTurn.PermissionModes.Length > 0,
            // Continue resumes natively, as the composer's main button does.
            Command.Continue => composing && model!.Submission.CanResume,
            Command.CompactContext => model is { CanCompact: true },
            Command.Stop => model is { CanCancel: true, Busy: false },
            Command.FocusTranscript => view != null,
            Command.CloseTab or Command.Rename => Editor.Focused.Selected != null,
            Command.GlobalShow or Command.GlobalNewConversation => false,
            _ => true,
        };
    }

    void ShowPalette(PaletteMode mode)
    {
        palette ??= new CommandPalette(PaletteItems, FocusComposer);
        if (Root.XamlRoot != null)
        {
            palette.Show(mode, Root.XamlRoot);
        }
    }

    List<PaletteItem> PaletteItems(PaletteMode mode)
    {
        var view = FocusedView;
        switch (mode)
        {
            case PaletteMode.Commands:
                var items = Commands.Table
                    .Where(entry => !Commands.IsGlobal(entry.Key) && entry.Key != Command.CommandPalette)
                    .Select(entry => new PaletteItem("command:" + entry.Key, entry.Value.Name, _ => Run(entry.Key),
                        Shortcut: Commands.Shortcut(entry.Key), Enabled: CanRun(entry.Key), Aliases: Commands.SearchTerms(entry.Key)))
                    .ToList();
                foreach (var (page, name, english) in settingsPages)
                {
                    items.Add(new PaletteItem("settings:" + page, "设置：" + name, _ => Shell.OpenSettings(page), "设置", Aliases: ["Settings: " + english, "preferences"]));
                }
                if (view?.HasQueued == true)
                {
                    items.Add(new PaletteItem("queue:last", "编辑最后一条排队消息", _ => view.TakeLastQueued(), "输入区", "Alt+↑", Aliases: ["Edit Last Queued Message", "queue"]));
                }
                return items;
            case PaletteMode.Conversations:
                var listing = Listing;
                return [.. listing.Conversations.Where(listed => !listed.Archived).Select(listed =>
                {
                    var summary = listed.Conversation;
                    var state = listing.Active.TryGetValue(summary.Id, out var active)
                        ? active.PendingInteractions > 0 ? "等待回应" : "运行中"
                        : listed.Unread ? listed.Failed ? "失败" : "未读" : null;
                    var where = listed.Project ?? summary.Cwd;
                    var detail = string.Join(" · ", new[] { where is { Length: > 0 } ? Listing.LastPart(where) : null, state }.OfType<string>());
                    var target = new Target(summary.RuntimeId, summary.Id, summary.Title.DisplayText());
                    return new PaletteItem("conversation:" + summary.Id, summary.Title.DisplayText(), newTab =>
                    {
                        Open(target, newTab ? Opening.NewTab : Opening.CurrentTab);
                        FocusComposer();
                    }, detail.Length > 0 ? detail : null);
                })];
            default:
                return view?.ChoiceItems(mode) ?? [];
        }
    }

    // GoToNextAttention goes to the next conversation that wants the user:
    // one waiting for a reply, then one that ended unseen, then one that
    // failed. A request in the conversation shown comes first.
    void GoToNextAttention()
    {
        if (FocusedView is { } view && view.Model.Interaction != null)
        {
            view.FocusInteraction();
            return;
        }
        var listing = Listing;
        var shown = ShownID;
        var waiting = listing.Active.Values.Where(active => active.PendingInteractions > 0 && active.ConversationId != shown).ToList();
        var candidates = listing.Conversations.Where(listed => listed.Conversation.Id != shown && listed.Conversation.ParentConversationId == null).ToList();
        static long Updated(ListedConversation conversation) => conversation.Conversation.UpdatedAtUnixMs ?? 0;
        Target? next = null;
        var asks = false;
        if (waiting.Count > 0)
        {
            var ids = waiting.Select(active => active.ConversationId).ToHashSet();
            var shownWaiting = candidates.Where(candidate => ids.Contains(candidate.Conversation.Id)).OrderByDescending(Updated).FirstOrDefault();
            next = shownWaiting != null
                ? new Target(shownWaiting.Conversation.RuntimeId, shownWaiting.Conversation.Id, shownWaiting.Conversation.Title.DisplayText())
                : new Target(waiting[0].ContextRuntimeId, waiting[0].ConversationId, "");
            asks = true;
        }
        else if ((candidates.Where(listed => listed.Unread && !listed.Failed).OrderByDescending(Updated).FirstOrDefault()
            ?? candidates.Where(listed => listed.Unread && listed.Failed).OrderByDescending(Updated).FirstOrDefault()) is { } ended)
        {
            next = new Target(ended.Conversation.RuntimeId, ended.Conversation.Id, ended.Conversation.Title.DisplayText());
        }
        if (next == null)
        {
            return;
        }
        Open(next, Opening.CurrentTab);
        FocusAttention(asks);
    }

    // FocusAttention puts the keyboard on the shown conversation's request
    // once it shows, else on its composer.
    internal void FocusAttention(bool asks)
    {
        if (asks && FocusedView is { } view)
        {
            view.FocusInteractionWhenShown();
        }
        else
        {
            FocusComposer();
        }
    }

    async void ShowShortcutHelp()
    {
        var panel = new StackPanel { Spacing = 2, MinWidth = 480 };
        void Section(string title, IEnumerable<(string Keys, string What)> rows)
        {
            panel.Children.Add(new TextBlock { Text = title, Style = Ui.Style("BodyStrongTextBlockStyle"), Margin = new Thickness(0, panel.Children.Count == 0 ? 0 : 12, 0, 4) });
            foreach (var (keys, what) in rows)
            {
                var row = new Grid { ColumnSpacing = 16 };
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(150) });
                row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                row.Children.Add(new TextBlock { Text = keys, Style = Ui.Style("SecondaryTextStyle") });
                var text = new TextBlock { Text = what, TextWrapping = TextWrapping.Wrap };
                Grid.SetColumn(text, 1);
                row.Children.Add(text);
                panel.Children.Add(row);
            }
        }
        var commands = Commands;
        Section("命令", Commands.Table.Where(entry => commands.Shortcut(entry.Key).Length > 0).Select(entry => (commands.Shortcut(entry.Key), entry.Value.Name)));
        Section("输入区", [
            (commands.SendKey, "发送；运行中加入发送队列"),
            (commands.SteerKey, "插入本轮；运行时不支持时加入队列"),
            (commands.NewLineKey, "换行"),
            ("Alt+↑", "编辑最后一条排队消息"),
            ("/", "Velune 命令"),
            ("@", "工作目录中的文件"),
            ("Esc", "关闭候选列表，否则停止生成"),
        ]);
        Section("命令面板", [("↑ ↓", "选择"), ("Enter", "执行"), ("Ctrl+Enter", "在新标签页打开会话"), ("Esc", "关闭")]);
        Section("审批与提问卡片", [
            ("1–9", "选择对应的选项"),
            ("Ctrl+Backspace", "取消请求"),
            ("Enter", "回答完毕后提交提问"),
            ("Tab", "在字段之间移动"),
            ("Esc", "回到输入区"),
        ]);
        Section("消息列表", [
            ("↑ ↓", "在消息之间移动"),
            ("Alt+↑ ↓", "在用户消息之间跳转"),
            ("Home End", "第一条、最后一条"),
            ("Enter Space", "展开或收起工作过程"),
            ("Ctrl+C", "拷贝所选消息"),
            ("Ctrl+Shift+C", "拷贝其中最后一个代码块"),
            ("E", "编辑所选的用户消息"),
        ]);
        Section("会话列表", [("↑ ↓", "选择并打开会话"), ("F2", "重命名"), ("Delete", "删除，需确认"), ("输入文字", "转到搜索框")]);
        var dialog = new ContentDialog
        {
            Title = "快捷键速查",
            Content = new ScrollViewer { Content = panel, MaxHeight = 520, Padding = new Thickness(0, 0, 16, 0) },
            CloseButtonText = "完成",
            SecondaryButtonText = "更改快捷键…",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await ShowDialog(dialog) == ContentDialogResult.Secondary)
        {
            Shell.OpenSettings(SettingsPage.General);
        }
    }
}
