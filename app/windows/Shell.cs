using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

// What the app's windows share: the core and its remote service, each
// device's conversation list, the settings window, pairing requests and the
// saved layout. The app quits with its last workspace window.
sealed class Shell
{
    public Core Core { get; }
    public Hub Hub { get; }
    public Listings Listings { get; }
    public Commands Commands { get; }
    readonly LayoutStore store;
    readonly SavedLayout saved;
    readonly List<WorkspaceWindow> windows = [];
    // The window the user worked in last, where requests from elsewhere show.
    WorkspaceWindow? latest;
    SettingsWindow? settings;
    ProblemsWindow? problems;
    AnalyticsWindow? analytics;
    LibraryWindow? library;
    // The user chose to quit with the library's edits unsaved.
    bool discardDrafts;
    readonly Notifier notifier;
    // The pairing attempts already put to the user.
    readonly HashSet<string> prompted = [];
    bool approving;
    bool quitting;

    public Shell(Core core, DispatcherQueue dispatcher)
    {
        Core = core;
        Hub = new Hub(core, dispatcher);
        Problems.Start(dispatcher, Hub.DeviceName);
        notifier = new Notifier(this, dispatcher);
        Listings = new Listings(core);
        Listings.Polled += notifier.Polled;
        Commands = new Commands(core.Home);
        store = new LayoutStore(core.Home, dispatcher);
        saved = store.Read();
        Hub.Changed += HubChanged;
    }

    // Start reopens the windows open when the app last quit, else one window.
    public void Start()
    {
        Hub.Start();
        // This PC's conversations notify whichever device the windows show.
        Listings.Follow(Device.Local, KeepLocal);
        notifier.Start();
        var reopened = saved.Windows.ToList();
        if (reopened.Count == 0)
        {
            reopened.Add(new WindowState { Tools = saved.Tools.Copy() });
        }
        foreach (var state in reopened)
        {
            Open(state);
        }
    }

    public WorkspaceWindow Open(WindowState state, Tab? adopted = null)
    {
        var window = new WorkspaceWindow(this, state, adopted);
        windows.Add(window);
        latest ??= window;
        window.Activated += (_, e) =>
        {
            if (e.WindowActivationState != WindowActivationState.Deactivated)
            {
                latest = window;
                notifier.Returned();
            }
        };
        window.AppWindow.Closing += (_, e) =>
        {
            if (!MayClose(window))
            {
                e.Cancel = true;
            }
        };
        window.Closed += (_, _) => Closed(window);
        window.Activate();
        return window;
    }

    // NewWindow opens a workspace with the latest tool layout.
    public WorkspaceWindow NewWindow(Device device) =>
        Open(new WindowState { Device = device.PeerId, Tools = saved.Tools.Copy() });

    // OpenConversationWindow opens a window of only the editor for a moved
    // tab or a conversation; its areas open again as the latest tools had them.
    public void OpenConversationWindow(Device device, Tab? tab, Target? target, Bounds? bounds)
    {
        var tools = saved.Tools.Copy();
        var open = Enum.GetValues<Area>().Where(area => !tools.Areas[area].Collapsed).ToList();
        foreach (var area in open)
        {
            tools.Areas[area].Collapsed = true;
        }
        tools.EditorOnly = open;
        // Its left area is the conversation's 关联 Agent rather than the list.
        tools.Tools[Tool.Linked] = new ToolState { Place = Place.Dock, Area = Area.Left };
        if (tools.Tools[Tool.List] is { Place: Place.Dock, Area: Area.Left } list)
        {
            list.Shown = false;
        }
        var editor = new PaneState();
        if (target != null)
        {
            editor.Tabs.Add(new TabState(target.RuntimeID, target.ConversationID, target.Title));
        }
        Open(new WindowState { Device = device.PeerId, Bounds = bounds, Tools = tools, Conversation = true, Editor = editor }, tab);
    }

    // WindowAt is the workspace window at a point of the screen, if any.
    public WorkspaceWindow? WindowAt(Windows.Graphics.PointInt32 point) => windows.FirstOrDefault(window => window.Contains(point));

    void Closed(WorkspaceWindow window)
    {
        windows.Remove(window);
        if (latest == window)
        {
            latest = windows.LastOrDefault();
        }
        if (windows.Count > 0)
        {
            Changed();
            return;
        }
        // The last window keeps its place in the saved layout, to reopen.
        quitting = true;
        var last = window.Save();
        store.Save(() => new SavedLayout { Tools = saved.Tools, Windows = [last] });
        store.Flush();
        settings?.Close();
        problems?.Close();
        analytics?.Close();
        library?.Close();
        Listings.Leave(Device.Local, KeepLocal);
        notifier.Stop();
        Core.Shutdown();
    }

    // Remember keeps a window's tool layout as the latest, which windows
    // without their own start from; “只显示编辑区” stays with the window.
    public void Remember(ToolLayout tools)
    {
        var kept = tools.Copy();
        if (kept.EditorOnly is { } collapsed)
        {
            foreach (var area in collapsed)
            {
                kept.Areas[area].Collapsed = false;
            }
            kept.EditorOnly = null;
        }
        saved.Tools.Tools = kept.Tools;
        saved.Tools.Areas = kept.Areas;
        Changed();
    }

    // Changed saves the layout a moment after a window's layout changed.
    public void Changed()
    {
        if (!quitting)
        {
            store.Save(() => new SavedLayout { Tools = saved.Tools, Windows = [.. windows.Select(window => window.Save())] });
        }
    }

    // Deleted closes the tabs that show a deleted conversation.
    public void Deleted(Device device, string conversationID)
    {
        foreach (var window in windows.Where(window => window.Device == device).ToList())
        {
            window.Deleted(conversationID);
        }
    }

    // OpenSettings shows the settings window at a page, reusing an open one.
    public void OpenSettings(SettingsPage page)
    {
        if (settings == null)
        {
            settings = new SettingsWindow(Core, Hub, Commands, OpenProblems, OpenLibrary, opened =>
            {
                // Opening a device shows it in the window used last.
                if (latest != null)
                {
                    latest.SwitchTo(opened);
                    latest.Activate();
                }
            });
            settings.Closed += (_, _) => settings = null;
        }
        settings.Show(page);
    }

    static void KeepLocal()
    {
    }

    // Flash asks for attention on the window used last.
    public void Flash()
    {
        if ((latest ?? windows.LastOrDefault()) is { } window)
        {
            Native.Flash(window);
        }
    }

    // InFront tells a workspace window is the one in front.
    public bool InFront => windows.Any(window => window.Foreground);

    // Viewing tells a conversation shows in front, so it needs no notification.
    public bool Viewing(Device device, string conversationID) =>
        windows.Any(window => window.Foreground && window.Device == device && window.ShownID == conversationID);

    // OpenNotified shows a notification's conversation in the window used
    // last, on the notification's device.
    public void OpenNotified(Device device, Target target)
    {
        if ((latest ?? windows.LastOrDefault()) is not { } window)
        {
            return;
        }
        if (window.Device != device)
        {
            window.SwitchTo(device);
        }
        window.Open(target, Opening.NewTab);
        window.Activate();
    }

    // OpenProblems shows the 问题 window, reusing an open one.
    public void OpenProblems()
    {
        if (problems == null)
        {
            problems = new ProblemsWindow();
            problems.Closed += (_, _) => problems = null;
        }
        problems.Activate();
    }

    // OpenLibrary shows the 资源库 window, reusing an open one.
    public void OpenLibrary()
    {
        if (library == null)
        {
            library = new LibraryWindow(Core);
            library.Closed += (_, _) => library = null;
        }
        library.Activate();
    }

    // RequestClose closes a workspace window, unless it is the last and the
    // user keeps the library's unsaved edits.
    public void RequestClose(WorkspaceWindow window)
    {
        if (MayClose(window))
        {
            window.Close();
        }
    }

    // MayClose tells whether a window can close now. Closing the last one
    // quits, so with unsaved library edits it asks first and closes the
    // window itself if the user discards them.
    bool MayClose(WorkspaceWindow window)
    {
        if (discardDrafts || windows.Count > 1 || Library.Unsaved.Count == 0)
        {
            return true;
        }
        AskQuit(window);
        return false;
    }

    async void AskQuit(WorkspaceWindow window)
    {
        var dialog = new ContentDialog
        {
            Title = "资源库中有未保存的修改",
            Content = Ui.Text("未保存：" + string.Join("、", Library.Unsaved) + "。退出后这些修改将丢失。"),
            PrimaryButtonText = "放弃修改并退出",
            SecondaryButtonText = "打开资源库",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Secondary,
        };
        switch (await window.ShowDialog(dialog))
        {
            case ContentDialogResult.Primary:
                discardDrafts = true;
                window.Close();
                break;
            case ContentDialogResult.Secondary:
                OpenLibrary();
                break;
        }
    }

    // OpenAnalytics shows the 分析 window, reusing an open one.
    public void OpenAnalytics()
    {
        if (analytics == null)
        {
            analytics = new AnalyticsWindow(Core, OpenProblems);
            analytics.Closed += (_, _) => analytics = null;
        }
        analytics.Activate();
    }

    void HubChanged()
    {
        if (!approving)
        {
            NextApproval();
        }
        settings?.HubChanged();
        foreach (var window in windows.ToList())
        {
            window.HubChanged();
        }
    }

    // NextApproval asks about the next pairing waiting for this PC's answer,
    // in the window used last.
    async void NextApproval()
    {
        var pairing = Hub.State?.IncomingPairings.FirstOrDefault(pairing =>
            pairing.Phase == BindingPairingPhase.AwaitingApproval && !prompted.Contains(pairing.AttemptId));
        if (pairing == null || latest is not { } window)
        {
            return;
        }
        prompted.Add(pairing.AttemptId);
        approving = true;
        window.Activate();
        var allow = await AskApproval(window, pairing);
        _ = Task.Run(() =>
        {
            try
            {
                var endpoint = Core.Remote();
                if (allow)
                {
                    endpoint.ApprovePairing(pairing.AttemptId, pairing.PeerId);
                }
                else
                {
                    endpoint.RejectPairing(pairing.AttemptId, pairing.PeerId);
                }
            }
            catch (BindingException)
            {
                // The next state shows where the pairing stands.
            }
        });
        approving = false;
        NextApproval();
    }

    // AskApproval asks whether a device may control this PC. Both devices show
    // the same code; only the user here can let the other one in. Neither
    // answer is the default, so a stray Enter or Esc grants nothing.
    async Task<bool> AskApproval(WorkspaceWindow window, BindingPairingStatus pairing)
    {
        var code = new TextBlock
        {
            Text = Hub.FormatCode(pairing.VerificationCode),
            FontFamily = Markdown.Mono(),
            FontSize = 30,
            HorizontalAlignment = HorizontalAlignment.Center,
            IsTextSelectionEnabled = true,
        };
        var dialog = new ContentDialog
        {
            Title = $"“{pairing.Name}”请求控制这台电脑",
            Content = new StackPanel
            {
                Spacing = 16,
                Children =
                {
                    new TextBlock { Text = "确认两台设备显示相同的核对码。允许后，对方可以查看和操作这台电脑上的会话。", TextWrapping = TextWrapping.Wrap },
                    code,
                },
            },
            PrimaryButtonText = "允许",
            SecondaryButtonText = "拒绝",
            DefaultButton = ContentDialogButton.None,
        };
        // The question stays until one of the buttons answers it.
        dialog.Closing += (_, e) => e.Cancel = e.Result == ContentDialogResult.None;
        // The code takes the dialog's focus, so no button is the default.
        dialog.Opened += (_, _) => code.Focus(FocusState.Programmatic);
        return await window.ShowDialog(dialog) == ContentDialogResult.Primary;
    }
}
