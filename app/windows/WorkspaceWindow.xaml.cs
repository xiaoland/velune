using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Velune.Bindings;
using Windows.Graphics;

namespace Velune;

// A workspace window: one device's conversations in an editor of split tab
// groups, with the tool windows docked around it (see TDD.md). Its state
// changes on the UI thread only.
public sealed partial class WorkspaceWindow : Window
{
    internal Shell Shell { get; }
    internal Device Device { get; private set; }
    internal Editor Editor { get; private set; }
    internal ToolWindows Tools { get; }
    internal ConversationList List { get; }
    readonly OutlineTool outline = new();
    readonly LinkedAgentsTool linked;
    // The docked composer shows the focused tab's composer.
    readonly Border composerHost = new();
    // A closed window throws on any further use; its last state stays.
    bool closed;
    WindowState? closedState;
    bool foreground = true;
    // Where the window was last when neither maximized nor minimized.
    Bounds? bounds;
    readonly SemaphoreSlim dialogs = new(1, 1);

    internal Hub Hub => Shell.Hub;
    internal Commands Commands => Shell.Commands;
    internal bool IsClosed => closed;
    internal Listing Listing => Shell.Listings.Of(Device);

    internal WorkspaceWindow(Shell shell, WindowState state, Tab? adopted = null)
    {
        InitializeComponent();
        Shell = shell;
        Device = new Device(state.Device);
        IsConversationWindow = state.Conversation;

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        if (MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop();
        }
        else
        {
            Root.Style = Ui.Style("WindowBackgroundStyle");
        }
        PlaceWindow(state);

        List = new ConversationList(this);
        linked = new LinkedAgentsTool(this);
        Tools = new ToolWindows(this, state.Tools, Body, Center,
            new() { [Area.Left] = LeftArea, [Area.Right] = RightArea, [Area.Bottom] = BottomArea },
            LeftColumn, RightColumn, BottomRow,
            new() { [Tool.List] = List, [Tool.Outline] = outline, [Tool.Composer] = composerHost, [Tool.Linked] = linked });
        Editor = adopted != null ? new Editor(this, EditorHost, adopted) : new Editor(this, EditorHost, state.Editor, state.Focus);
        if (adopted != null)
        {
            Adopt(adopted);
        }
        Pinned = state.Pinned;
        Tools.Build();

        AppTitleBar.PaneToggleRequested += (_, _) => Tools.ToggleArea(Area.Left);
        AppTitleBar.BackRequested += (_, _) => Back(Editor.Focused.Selected, true);
        InstallCommands();
        Commands.Changed += InstallCommands;
        NewButton.Click += (_, _) => Open(new Target("", "", ""), Opening.CurrentTab);
        SettingsButton.Click += (_, _) => Shell.OpenSettings(SettingsPage.Keep);
        // Failures go to 问题; its button shows while there are some.
        RightButtons.Children.Insert(0, new ProblemsButton(Shell.OpenProblems));
        DeviceMenu.Opening += (_, _) => FillDeviceMenu();
        MoreMenu.Opening += (_, _) => FillMoreMenu();
        LayoutMenu.Opening += (_, _) =>
        {
            LayoutMenu.Items.Clear();
            foreach (var item in Tools.LayoutItems())
            {
                LayoutMenu.Items.Add(item);
            }
        };
        Root.ActualThemeChanged += (_, _) => ShowMarks();
        Activated += (_, e) => SetForeground(e.WindowActivationState != WindowActivationState.Deactivated);
        AppWindow.Changed += (_, e) =>
        {
            if (!closed && (e.DidPositionChange || e.DidSizeChange || e.DidPresenterChange))
            {
                Track();
                Tools.OwnerMoved();
            }
        };
        // Closing stops everything that would update the window.
        Closed += (_, _) =>
        {
            closedState = Save();
            closed = true;
            Commands.Changed -= InstallCommands;
            Shell.Listings.Leave(Device, ListingChanged);
            Hub.SetActivity(Device, Device.Local, false);
            outline.Follow(null);
            linked.Follow(null);
            Tools.CloseAll();
            Editor.CloseAll();
        };

        ShowMarks();
        Shell.Listings.Follow(Device, ListingChanged);
        Hub.SetActivity(Device.Local, Device, true);
        foreach (var pane in Editor.Panes())
        {
            if (pane.Selected is { } tab)
            {
                Show(tab);
            }
        }
        FocusChanged();
    }

    // InstallCommands puts the commands' shortcuts on the window, and in
    // the tips that name them.
    void InstallCommands()
    {
        Commands.Install(Root, Run);
        string Tip(string name, Command command) => Commands.Shortcut(command) is { Length: > 0 } keys ? $"{name}（{keys}）" : name;
        ToolTipService.SetToolTip(NewButton, Tip("新会话", Command.NewConversation));
        ToolTipService.SetToolTip(SettingsButton, Tip("设置", Command.Settings));
    }

    // Size sizes a window in effective pixels and keeps it from shrinking past
    // its minimum.
    internal static void Size(Window window, int width, int height, int minWidth, int minHeight)
    {
        var scale = Native.Scale(window);
        window.AppWindow.Resize(new SizeInt32((int)(width * scale), (int)(height * scale)));
        if (window.AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = (int)(minWidth * scale);
            presenter.PreferredMinimumHeight = (int)(minHeight * scale);
        }
    }

    // Place puts the window where it was, if that is still on a display.
    void PlaceWindow(WindowState state)
    {
        Size(this, 1180, 780, 720, 480);
        if (state.Bounds is { } saved)
        {
            var area = DisplayArea.GetFromRect(new RectInt32(saved.X, saved.Y, saved.Width, saved.Height), DisplayAreaFallback.None);
            if (area != null)
            {
                AppWindow.MoveAndResize(new RectInt32(saved.X, saved.Y, saved.Width, saved.Height));
                bounds = saved;
            }
        }
        if (state.Maximized && AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.Maximize();
        }
    }

    void Track()
    {
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Restored })
        {
            bounds = new Bounds(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);
        }
        LayoutChanged();
    }

    internal WindowState Save() => closedState ?? new WindowState
    {
        Device = Device.PeerId,
        Bounds = bounds,
        Maximized = AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Maximized },
        Pinned = Pinned,
        Editor = Editor.Save(),
        Focus = Editor.FocusIndex,
        Tools = Tools.Layout.Copy(),
        Conversation = IsConversationWindow,
    };

    // Pinned keeps the window above other windows.
    internal bool Pinned
    {
        get => AppWindow.Presenter is OverlappedPresenter { IsAlwaysOnTop: true };
        set
        {
            if (AppWindow.Presenter is OverlappedPresenter presenter && presenter.IsAlwaysOnTop != value)
            {
                presenter.IsAlwaysOnTop = value;
                LayoutChanged();
            }
        }
    }

    internal void LayoutChanged()
    {
        if (!closed)
        {
            Shell.Changed();
        }
    }

    // ToolsChanged keeps the tool layout as the latest for new windows.
    internal void ToolsChanged()
    {
        if (closed)
        {
            return;
        }
        // A conversation window's tools are its own.
        if (!IsConversationWindow)
        {
            Shell.Remember(Tools.Layout);
        }
        LayoutChanged();
        ShowOutlineButtons();
    }

    void ShowMarks()
    {
        TitleIcon.ImageSource = Core.Brand("small", Root.ActualTheme == ElementTheme.Dark);
    }

    void SetForeground(bool active)
    {
        if (closed)
        {
            return;
        }
        foreground = active;
        Hub.SetActivity(Device, Device, active);
        NoteSeen();
    }

    internal void HubChanged()
    {
        if (closed)
        {
            return;
        }
        // The device was removed or its pairing undone: return to this PC.
        if (!Device.IsLocal && Hub.State != null && !Hub.Devices().Any(contact => contact.PeerId == Device.PeerId))
        {
            SwitchTo(Device.Local);
        }
        Render();
        List.Render();
    }

    void ListingChanged()
    {
        if (closed)
        {
            return;
        }
        var configuration = Listing.Configuration;
        foreach (var tab in Editor.Tabs())
        {
            if (tab.View is { } view)
            {
                view.Model.Configuration = configuration;
            }
        }
        List.ListChanged();
        NoteSeen();
        Render();
    }

    // ShowDialog shows one dialog at a time over the window.
    internal async Task<ContentDialogResult> ShowDialog(ContentDialog dialog)
    {
        await dialogs.WaitAsync();
        try
        {
            dialog.XamlRoot = Root.XamlRoot;
            return await dialog.ShowAsync();
        }
        finally
        {
            dialogs.Release();
        }
    }

    internal Task<string?> PickFolder() => Native.PickFolder(this);

    internal void OpenSettings(SettingsPage page) => Shell.OpenSettings(page);

    // Foreground tells the window is the one in front.
    internal bool Foreground => foreground && !closed;

    // ShownID is the conversation the focused tab shows.
    internal string? ShownID => Editor.Focused.Selected?.Target is { Unsent: false } target ? target.ConversationID : null;

    ConversationView? FocusedView => Editor.Focused.Selected?.View;

    // Show makes a tab's view, the first time the tab shows.
    internal void Show(Tab tab)
    {
        if (tab.View == null)
        {
            var view = new ConversationView(new Conversation(Shell.Core, Device, tab.Target, Listing.Configuration), this);
            tab.View = view;
            tab.Host.Child = view;
            Follow(tab);
            PlaceComposers();
        }
        TabChanged(tab);
    }

    // Follow shows the changes of a tab's view on the tab.
    void Follow(Tab tab)
    {
        if (tab.View is not { } view)
        {
            return;
        }
        void Changed() => TabChanged(tab);
        view.Model.Changed += Changed;
        tab.Unfollow = () => view.Model.Changed -= Changed;
    }

    // TabChanged shows a tab's title and state on its tab, and in the window
    // when it is the focused one.
    void TabChanged(Tab tab)
    {
        if (closed)
        {
            return;
        }
        if (tab.View is { } view && !view.Model.Target.Unsent)
        {
            // A conversation sent for the first time is the tab's from now on.
            tab.Target = view.Model.Target;
        }
        var title = tab.Title;
        tab.Item.Header = title;
        ToolTipService.SetToolTip(tab.Item, title);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(tab.Item, title);
        var model = tab.View?.Model;
        var state = model?.Interaction != null ? "\uE7BA" : model?.Working == true ? "\uE895" : "";
        if (state != tab.State)
        {
            tab.State = state;
            tab.Item.IconSource = state.Length > 0 ? new FontIconSource { Glyph = state } : null;
        }
        if (tab == Editor.Focused.Selected)
        {
            Render();
            ShowOutlineButtons();
        }
    }

    void Render()
    {
        if (closed)
        {
            return;
        }
        var deviceName = Hub.DeviceName(Device) + (Device.IsLocal ? "（本机）" : "");
        DeviceButton.Content = deviceName;
        var title = Editor.Focused.Selected?.Title;
        Title = title == null ? $"Velune — {deviceName}" : $"{title} — {deviceName} — Velune";
        AppTitleBar.IsBackButtonVisible = Editor.Focused.Selected is { Back.Count: > 0 };
        NewButton.IsEnabled = Listing.Configuration.Runtimes.Count > 0;
    }

    // FocusChanged follows the focused pane: the docked tools show its tab.
    internal void FocusChanged()
    {
        if (closed)
        {
            return;
        }
        outline.Follow(FocusedView);
        linked.Follow(FocusedView);
        PlaceComposers();
        List.ShowSelection();
        ShowOutlineButtons();
        NoteSeen();
        Render();
    }

    // ShowOutlineButtons shows the outline's button on each pane while the
    // outline is one, checked while its tab's list is folded to it.
    internal void ShowOutlineButtons()
    {
        var button = Tools.Layout.Tools[Tool.Outline].Place == Place.Button;
        var linkedButton = Tools.Layout.Tools[Tool.Linked].Place == Place.Button;
        foreach (var pane in Editor.Panes())
        {
            pane.OutlineButton.Visibility = button ? Visibility.Visible : Visibility.Collapsed;
            var view = pane.Selected?.View;
            pane.LinkedButton.Visibility = linkedButton && LinkedAgentsTool.Has(view) ? Visibility.Visible : Visibility.Collapsed;
            pane.OutlineButton.IsEnabled = view?.HasOutline == true;
            pane.OutlineButton.IsChecked = view?.OutlineFolded == true;
        }
    }

    // PlaceComposers puts each tab's composer in its pane, or the focused
    // tab's in the composer's tool window when it has one.
    internal void PlaceComposers()
    {
        var inPane = Tools.Layout.Tools[Tool.Composer].Place == Place.InPane;
        composerHost.Child = null;
        foreach (var tab in Editor.Tabs())
        {
            tab.View?.ComposerInPane(inPane);
        }
        if (!inPane && FocusedView is { } view)
        {
            composerHost.Child = view.ComposerArea;
        }
    }

    // NoteSeen marks the focused tab's conversation read once it shows in the
    // active window.
    void NoteSeen()
    {
        if (foreground && ShownID is { } id && !Shell.Listings.MarkingRead(Device, id)
            && Listing.Conversations.FirstOrDefault(listed => listed.Conversation.Id == id) is { Unread: true } listed)
        {
            Shell.Listings.MarkRead(Device, listed);
        }
    }

    // Open shows a conversation. A tab of this window that shows it already
    // is chosen instead, so one conversation has one tab here.
    internal void Open(Target target, Opening how)
    {
        if (!target.Unsent && Editor.Find(target.ConversationID) is { } shown)
        {
            Editor.Select(shown);
            return;
        }
        switch (how)
        {
            case Opening.NewTab:
                Editor.NewTab(Editor.Focused, target);
                break;
            case Opening.SplitRight or Opening.SplitDown:
                Editor.SplitWith(Editor.Focused, how == Opening.SplitDown, new Tab(target));
                break;
            default:
                if (Editor.Focused.Selected is { } tab)
                {
                    Navigate(tab, target, true);
                }
                else
                {
                    Editor.NewTab(Editor.Focused, target);
                }
                break;
        }
    }

    // Navigate shows another conversation in a tab; back returns to this one.
    void Navigate(Tab tab, Target target, bool remember)
    {
        if (remember && !tab.Target.Unsent)
        {
            tab.Back.Push(tab.Target);
            tab.Forward.Clear();
        }
        tab.Unfollow?.Invoke();
        tab.Unfollow = null;
        tab.View?.Close();
        tab.View = null;
        tab.Host.Child = null;
        tab.Target = target;
        Show(tab);
        FocusChanged();
        LayoutChanged();
    }

    void Back(Tab? tab, bool back)
    {
        if (tab == null || (back ? tab.Back : tab.Forward).Count == 0)
        {
            return;
        }
        var target = (back ? tab.Back : tab.Forward).Pop();
        if (!tab.Target.Unsent)
        {
            (back ? tab.Forward : tab.Back).Push(tab.Target);
        }
        Navigate(tab, target, false);
    }

    // OpenLinked opens a conversation a transcript links to: a collaborator,
    // a native sub-agent or its parent, which the list may not show.
    // ShowLinked shows 关联 Agent at a pane's button, for its tab.
    internal void ShowLinked(Pane pane)
    {
        if (pane.Selected?.View is not { } view || !LinkedAgentsTool.Has(view))
        {
            return;
        }
        var shown = new LinkedAgentsTool(this) { Width = 320, MaxHeight = 420 };
        shown.Follow(view);
        var flyout = new Flyout { Content = shown, Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.BottomEdgeAlignedRight };
        flyout.Closed += (_, _) => shown.Follow(null);
        flyout.ShowAt(pane.LinkedButton);
    }

    internal async void OpenLinked(string conversationID, BindingLinkedConversationOpening? how = null)
    {
        var target = Device;
        BindingListedConversation? found;
        try
        {
            found = await Task.Run(() => Shell.Core.Application(target).FindConversation(conversationID));
        }
        catch (Exception failure)
        {
            Problems.Report("打开关联会话", failure, target);
            return;
        }
        if (closed || target != Device)
        {
            return;
        }
        if (found == null)
        {
            Problems.Report("打开关联会话", "这台设备上找不到该会话。", target);
            return;
        }
        var linked = new Target(found.Conversation.RuntimeId, found.Conversation.Id, found.Conversation.Title);
        switch (how ?? Listing.Configuration.LinkedOpening)
        {
            case BindingLinkedConversationOpening.CurrentTab:
                Open(linked, Opening.CurrentTab);
                break;
            case BindingLinkedConversationOpening.NewWindow when Editor.Find(linked.ConversationID) == null:
                Shell.OpenConversationWindow(Device, null, linked, null);
                break;
            case BindingLinkedConversationOpening.Sheet when Editor.Find(linked.ConversationID) == null:
                ShowSheet(linked);
                break;
            default:
                Open(linked, Opening.NewTab);
                break;
        }
    }

    // ShowSheet opens a linked conversation over the window, as a tab with
    // its own composer; it can move on, as it is, to a tab or a window.
    async void ShowSheet(Target target)
    {
        var view = new ConversationView(new Conversation(Shell.Core, Device, target, Listing.Configuration), this)
        {
            Width = Math.Max(Root.ActualWidth * 0.8, 480),
            Height = Math.Max(Root.ActualHeight * 0.75, 360),
        };
        var dialog = new ContentDialog
        {
            Title = target.Title,
            Content = view,
            PrimaryButtonText = "转到标签页",
            SecondaryButtonText = "在新窗口中打开",
            CloseButtonText = "关闭",
            DefaultButton = ContentDialogButton.None,
        };
        dialog.Resources["ContentDialogMaxWidth"] = 4000.0;
        dialog.Resources["ContentDialogMaxHeight"] = 4000.0;
        var result = await ShowDialog(dialog);
        dialog.Content = null;
        if (closed || result == ContentDialogResult.None)
        {
            view.Close();
            return;
        }
        view.Width = double.NaN;
        view.Height = double.NaN;
        var tab = new Tab(view.Model.Target) { View = view };
        tab.Host.Child = view;
        if (result == ContentDialogResult.Secondary)
        {
            Shell.OpenConversationWindow(Device, tab, null, null);
            return;
        }
        Follow(tab);
        var pane = Editor.Focused;
        Editor.Insert(pane, tab, pane.Selected is { } selected ? pane.View.TabItems.IndexOf(selected.Item) + 1 : pane.Count);
    }

    // SaveTitle renames a conversation in its runtime, showing the title at once.
    internal void SaveTitle(BindingListedConversation listed, string title)
    {
        var conversations = Listing.Conversations;
        if (conversations.IndexOf(listed) is var at and >= 0)
        {
            conversations[at] = listed with { Conversation = listed.Conversation with { Title = title } };
        }
        Shell.Listings.Notify(Device);
        Shell.Listings.Write(Device, "重命名会话", application => application.RenameConversation(listed.Conversation.RuntimeId, listed.Conversation.Id, title));
    }

    // Reread reads the list and the open conversations again.
    internal void Reread()
    {
        Shell.Listings.Reread(Device);
        foreach (var tab in Editor.Tabs())
        {
            tab.View?.Model.Load();
        }
    }

    // Deleted closes the tabs that show a deleted conversation.
    internal void Deleted(string conversationID)
    {
        if (Editor.Find(conversationID) is { } tab)
        {
            Editor.Close(tab, false);
        }
    }

    // SwitchTo shows another device in this window, from one new tab.
    internal void SwitchTo(Device next)
    {
        if (next == Device || closed)
        {
            return;
        }
        Hub.SetActivity(Device, next, foreground);
        Shell.Listings.Leave(Device, ListingChanged);
        Editor.CloseAll();
        Device = next;
        Editor = new Editor(this, EditorHost, new PaneState(), 0);
        Shell.Listings.Follow(Device, ListingChanged);
        List.DeviceChanged();
        foreach (var pane in Editor.Panes())
        {
            if (pane.Selected is { } tab)
            {
                Show(tab);
            }
        }
        FocusChanged();
        LayoutChanged();
    }

    void FillDeviceMenu()
    {
        DeviceMenu.Items.Clear();
        void Add(string name, Device target)
        {
            var item = new RadioMenuFlyoutItem { Text = name, GroupName = "device", IsChecked = target == Device };
            item.Click += (_, _) => SwitchTo(target);
            DeviceMenu.Items.Add(item);
        }
        Add(Hub.DeviceName(Device.Local) + "（本机）", Device.Local);
        foreach (var contact in Hub.Devices())
        {
            Add(contact.Name, new Device(contact.PeerId));
        }
        DeviceMenu.Items.Add(new MenuFlyoutSeparator());
        DeviceMenu.Items.Add(Ui.MenuItem("添加设备…", () => Shell.OpenSettings(SettingsPage.Devices)));
    }

    // FillTabMenu offers what may be done with a tab and the window's layout.
    internal void FillTabMenu(MenuFlyout menu, Tab tab)
    {
        menu.Items.Clear();
        var pane = Editor.PaneOf(tab);
        if (pane == null)
        {
            return;
        }
        if (Listing.Conversations.FirstOrDefault(listed => !tab.Target.Unsent && listed.Conversation.Id == tab.Target.ConversationID) is { } listed)
        {
            var rename = Ui.MenuItem("重命名…", () => RenameTab(listed));
            rename.IsEnabled = listed.Conversation.CanRename;
            menu.Items.Add(rename);
        }
        menu.Items.Add(Ui.MenuItem("关闭", () => Editor.Close(tab, false)));
        var others = Ui.MenuItem("关闭其他", () => Editor.CloseOthers(tab));
        others.IsEnabled = Editor.Tabs().Skip(1).Any();
        menu.Items.Add(others);
        menu.Items.Add(new MenuFlyoutSeparator());
        if (pane.Count > 1)
        {
            menu.Items.Add(Ui.MenuItem("移到右侧拆分", () => Editor.MoveToSplit(tab, false)));
            menu.Items.Add(Ui.MenuItem("移到下方拆分", () => Editor.MoveToSplit(tab, true)));
        }
        else
        {
            menu.Items.Add(Ui.MenuItem("向右拆分", () => Editor.SplitWith(pane, false, new Tab(new Target("", "", "")))));
            menu.Items.Add(Ui.MenuItem("向下拆分", () => Editor.SplitWith(pane, true, new Tab(new Target("", "", "")))));
        }
        menu.Items.Add(Ui.MenuItem("移到新窗口", () => MoveToNewWindow(tab, null)));
        menu.Items.Add(new MenuFlyoutSeparator());
        var layout = new MenuFlyoutSubItem { Text = "布局" };
        foreach (var item in Tools.LayoutItems())
        {
            layout.Items.Add(item);
        }
        menu.Items.Add(layout);
    }

    // RenameTab asks for a conversation's new title.
    async void RenameTab(BindingListedConversation listed)
    {
        var input = new TextBox { Text = listed.Conversation.Title };
        var dialog = new ContentDialog
        {
            Title = "重命名会话",
            Content = input,
            PrimaryButtonText = "重命名",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };
        input.Loaded += (_, _) => input.SelectAll();
        if (await ShowDialog(dialog) == ContentDialogResult.Primary && input.Text.Trim() is { Length: > 0 } title && title != listed.Conversation.Title)
        {
            SaveTitle(listed, title);
        }
    }

    void Run(Command command)
    {
        var tab = Editor.Focused.Selected;
        switch (command)
        {
            case Command.NewConversation:
                Open(new Target("", "", ""), Opening.CurrentTab);
                break;
            case Command.NewTab:
                Editor.NewTab(Editor.Focused, null);
                break;
            case Command.NewWindow:
                Shell.NewWindow(Device);
                break;
            case Command.CloseWindow:
                Shell.RequestClose(this);
                break;
            case Command.CloseTab:
                if (tab != null)
                {
                    Editor.Close(tab, true);
                }
                break;
            case Command.SplitRight or Command.SplitDown:
                var vertical = command == Command.SplitDown;
                if (tab != null && Editor.Focused.Count > 1)
                {
                    Editor.MoveToSplit(tab, vertical);
                }
                else
                {
                    Editor.SplitWith(Editor.Focused, vertical, new Tab(new Target("", "", "")));
                }
                break;
            case Command.PreviousPane:
                Editor.StepPane(-1);
                break;
            case Command.NextPane:
                Editor.StepPane(1);
                break;
            case Command.PreviousTab:
                Editor.StepTab(-1);
                break;
            case Command.NextTab:
                Editor.StepTab(1);
                break;
            case Command.Back:
                Back(tab, true);
                break;
            case Command.Forward:
                Back(tab, false);
                break;
            case Command.ToggleLeft:
                Tools.ToggleArea(Area.Left);
                break;
            case Command.ToggleRight:
                Tools.ToggleArea(Area.Right);
                break;
            case Command.ToggleBottom:
                Tools.ToggleArea(Area.Bottom);
                break;
            case Command.EditorOnly:
                Tools.ToggleEditorOnly();
                break;
            case Command.ShowList:
                Tools.ToggleTool(Tool.List);
                break;
            case Command.ShowOutline:
                Tools.ToggleTool(Tool.Outline);
                break;
            case Command.Problems:
                Shell.OpenProblems();
                break;
            case Command.Analytics:
                Shell.OpenAnalytics();
                break;
            case Command.Library:
                Shell.OpenLibrary();
                break;
            case Command.ShowLinked:
                Tools.ToggleTool(Tool.Linked);
                break;
            case Command.FocusComposer:
                FocusComposer();
                break;
            case Command.FindConversation:
                if (!Tools.Layout.Visible(Tool.List))
                {
                    Tools.SetShown(Tool.List, true);
                }
                List.FocusSearch();
                break;
            case Command.Rename:
                List.RenameShown();
                break;
            case Command.Settings:
                Shell.OpenSettings(SettingsPage.Keep);
                break;
        }
    }

    // FillMoreMenu lists this PC's own windows besides settings.
    void FillMoreMenu()
    {
        MoreMenu.Items.Clear();
        foreach (var (command, name) in new[] { (Command.Library, "资源库…"), (Command.Analytics, "分析…"), (Command.Problems, "问题…") })
        {
            var item = Ui.MenuItem(name, () => Run(command));
            if (Commands.Bound(command) is { } keys)
            {
                item.KeyboardAcceleratorTextOverride = keys.ToString();
            }
            MoreMenu.Items.Add(item);
        }
    }

    // FocusComposer goes to the focused tab's composer, opening the area it
    // is docked in.
    void FocusComposer()
    {
        var state = Tools.Layout.Tools[Tool.Composer];
        if (state.Place == Place.Dock && !Tools.Layout.Visible(Tool.Composer))
        {
            Tools.SetShown(Tool.Composer, true);
        }
        FocusedView?.FocusInput();
    }
}
