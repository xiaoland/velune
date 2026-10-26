using System.ComponentModel;
using System.Diagnostics;
using System.Text.RegularExpressions;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;
using Windows.System;
using Windows.UI.Core;
using DispatcherQueue = Microsoft.UI.Dispatching.DispatcherQueue;
using DispatcherQueuePriority = Microsoft.UI.Dispatching.DispatcherQueuePriority;

namespace Velune;

// The view's keyboard (see docs/prd/keyboard.md): the composer's send
// and steer keys, the last queued message, the / and @ lists, the
// request card's numbers and the transcript's rows.
public sealed partial class ConversationView
{
    // Set while an input method composes, whose keys are its own.
    bool imeComposing;
    // Each queued message's in-place editor, by ID, for Alt+↑.
    readonly Dictionary<string, Action> queueEdits = [];
    // The / or @ list: what it offers, from where in the text, and the text
    // Esc dismissed it for.
    sealed record Completion(string Id, string Title, string? Detail, string? Shortcut, bool Enabled);
    List<Completion> completions = [];
    bool completingFiles;
    int completionStart;
    string? dismissedCompletion;
    // The request card's first answer, and when the keyboard was asked to
    // go to a request not shown yet.
    Control? firstAnswer;
    DateTime? interactionFocusRequested;
    // The message each transcript row shows.
    readonly Dictionary<UIElement, BindingMessage> rowMessages = [];

    // The Velune commands / offers, by prefix; other / text goes to the runtime as typed.
    static readonly (string Name, Command Command)[] slashCommands =
    [
        ("model", Command.ChooseModel),
        ("effort", Command.ChooseEffort),
        ("permission", Command.ChoosePermission),
        ("runtime", Command.ChooseRuntime),
        ("new", Command.NewConversation),
        ("continue", Command.Continue),
        ("stop", Command.Stop),
        ("attach", Command.Attach),
        ("open", Command.OpenConversation),
        ("find", Command.FindConversation),
        ("commands", Command.CommandPalette),
        ("keys", Command.ShortcutHelp),
    ];

    void AttachKeyboard()
    {
        Input.TextCompositionStarted += (_, _) => imeComposing = true;
        Input.TextCompositionEnded += (_, _) => imeComposing = false;
        Input.TextChanged += (_, _) => UpdateCompletion();
        Input.SelectionChanged += (_, _) => UpdateCompletion();
        CompletionList.ItemClick += (_, e) =>
        {
            if (CompletionList.Items.IndexOf(e.ClickedItem) is var index and >= 0)
            {
                AcceptCompletion(index);
            }
        };
        InteractionCard.KeyDown += CardKey;
        Transcript.PreviewKeyDown += TranscriptKey;
        window.Shell.Commands.Changed += RenderComposer;
    }

    static bool Down(VirtualKey key) => InputKeyboardSource.GetKeyStateForCurrentThread(key).HasFlag(CoreVirtualKeyStates.Down);

    static bool Plain => !Down(VirtualKey.Control) && !Down(VirtualKey.Shift) && !Down(VirtualKey.Menu);

    static int? Digit(VirtualKey key) => key switch
    {
        >= VirtualKey.Number1 and <= VirtualKey.Number9 => key - VirtualKey.Number0,
        >= VirtualKey.NumberPad1 and <= VirtualKey.NumberPad9 => key - VirtualKey.NumberPad0,
        _ => null,
    };

    // Composing tells the view takes new messages: a sub-agent's does not.
    internal bool Composing => ComposerPanel.Visibility == Visibility.Visible;

    void InputKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (CompletionKey(e.Key) || ComposerKey(e.Key))
        {
            e.Handled = true;
        }
    }

    // ComposerKey handles the composer's keys as the send mode has them:
    // sending or queueing, steering, the last queued message, and Esc,
    // which stops the running turn.
    bool ComposerKey(VirtualKey key)
    {
        if (imeComposing)
        {
            return false;
        }
        var control = Down(VirtualKey.Control);
        var shift = Down(VirtualKey.Shift);
        var alt = Down(VirtualKey.Menu);
        var plain = !control && !shift && !alt;
        switch (key)
        {
            case VirtualKey.Enter when !alt:
                if (window.Shell.Commands.SendMode == SendMode.Enter)
                {
                    if (plain)
                    {
                        Send();
                        return true;
                    }
                    if (control)
                    {
                        Steer();
                        return true;
                    }
                    return false;
                }
                if (control)
                {
                    if (shift)
                    {
                        Steer();
                    }
                    else
                    {
                        Send();
                    }
                    return true;
                }
                return false;
            case VirtualKey.Up when alt && !control && !shift:
                return TakeLastQueued();
            case VirtualKey.Escape when plain && Model.CanCancel && !Model.Busy:
                Model.Stop();
                return true;
        }
        return false;
    }

    internal bool HasQueued => Model.FollowUp?.Queue is { Length: > 0 };

    // TakeLastQueued edits the last queued message: into the empty composer,
    // out of the queue, or in its own editor when the composer holds a draft.
    internal bool TakeLastQueued()
    {
        if (Model.FollowUp?.Queue is not { Length: > 0 } queue || Model.Busy)
        {
            return false;
        }
        var last = queue[^1];
        if (Input.Text.Length > 0 || Model.Attached.Count > 0)
        {
            if (queueEdits.TryGetValue(last.Id, out var edit))
            {
                edit();
            }
            return true;
        }
        TakeBack(last);
        return true;
    }

    async void TakeBack(BindingQueuedMessage message)
    {
        if (await Model.TakeQueued(message) && Input.Text.Length == 0)
        {
            SetInput(message.Text);
            Input.SelectionStart = Input.Text.Length;
            FocusInput();
        }
    }

    internal async void Attach()
    {
        foreach (var path in await Native.PickFiles(window))
        {
            await AttachFile(path);
        }
        FocusInput();
    }

    internal async void ContinueTurn()
    {
        ScrollToEnd();
        await Model.Continue();
    }

    // UpdateCompletion offers Velune's commands for a / alone in the
    // composer, and the working directory's files for the @ word at the caret.
    void UpdateCompletion()
    {
        var text = Input.Text;
        if (imeComposing || Input.SelectionLength > 0 || text == dismissedCompletion)
        {
            ShowCompletions([]);
            return;
        }
        if (text.StartsWith('/') && !text.Any(char.IsWhiteSpace))
        {
            var query = text[1..].ToLowerInvariant();
            var commands = window.Shell.Commands;
            completingFiles = false;
            completionStart = 0;
            ShowCompletions([.. slashCommands.Where(entry => entry.Name.StartsWith(query, StringComparison.Ordinal)).Select(entry =>
                new Completion(entry.Name, "/" + entry.Name, Commands.Table[entry.Command].Name, commands.Shortcut(entry.Command), window.CanRun(entry.Command)))]);
            return;
        }
        var caret = Math.Min(Input.SelectionStart, text.Length);
        var start = caret;
        while (start > 0 && !char.IsWhiteSpace(text[start - 1]))
        {
            start--;
        }
        if (start >= caret || text[start] != '@' || FileDirectory() is not { } directory)
        {
            ShowCompletions([]);
            return;
        }
        completingFiles = true;
        completionStart = start;
        var files = WorkingFiles.Get(directory, DispatcherQueue, UpdateCompletion);
        ShowCompletions(files == null
            ? [new Completion("", "正在读取工作目录…", null, null, false)]
            : [.. WorkingFiles.Rank(files, text[(start + 1)..caret]).Select(path => new Completion(path, Path.GetFileName(path), Path.GetDirectoryName(path), null, true))]);
    }

    // FileDirectory is where @ finds files: the conversation's working
    // directory on this PC.
    string? FileDirectory()
    {
        if (!Model.Device.IsLocal)
        {
            return null;
        }
        var cwd = Model.Target.Unsent ? Model.Cwd : Model.Snapshot?.Conversation.Cwd;
        return cwd is { Length: > 0 } && Directory.Exists(cwd) ? cwd : null;
    }

    void ShowCompletions(List<Completion> wanted)
    {
        if (wanted.SequenceEqual(completions))
        {
            return;
        }
        // Typing on keeps the chosen row while it still matches.
        var chosen = completions.ElementAtOrDefault(CompletionList.SelectedIndex)?.Id;
        completions = wanted;
        CompletionList.Items.Clear();
        foreach (var completion in wanted)
        {
            CompletionList.Items.Add(CompletionRow(completion));
        }
        CompletionCard.Visibility = Visible(wanted.Count > 0);
        if (wanted.Count > 0)
        {
            CompletionList.SelectedIndex = Math.Max(wanted.FindIndex(completion => completion.Id == chosen), 0);
        }
    }

    static Grid CompletionRow(Completion completion)
    {
        var row = new Grid { ColumnSpacing = 8, Opacity = completion.Enabled ? 1 : 0.5 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var text = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        text.Children.Add(new TextBlock { Text = completion.Title, TextTrimming = TextTrimming.CharacterEllipsis });
        if (completion.Detail is { Length: > 0 } detail)
        {
            text.Children.Add(new TextBlock { Text = detail, Style = Ui.Style("SecondaryTextStyle"), TextTrimming = TextTrimming.CharacterEllipsis });
        }
        row.Children.Add(text);
        if (completion.Shortcut is { Length: > 0 } shortcut)
        {
            var keys = new TextBlock { Text = shortcut, Style = Ui.Style("SecondaryTextStyle") };
            Grid.SetColumn(keys, 1);
            row.Children.Add(keys);
        }
        return row;
    }

    // CompletionKey takes ↑↓, Enter, Tab and Esc while the list shows.
    bool CompletionKey(VirtualKey key)
    {
        if (completions.Count == 0 || imeComposing || !Plain)
        {
            return false;
        }
        switch (key)
        {
            case VirtualKey.Up or VirtualKey.Down:
                CompletionList.SelectedIndex = (CompletionList.SelectedIndex + (key == VirtualKey.Up ? -1 : 1) + completions.Count) % completions.Count;
                CompletionList.ScrollIntoView(CompletionList.SelectedItem);
                return true;
            case VirtualKey.Enter or VirtualKey.Tab:
                AcceptCompletion(CompletionList.SelectedIndex);
                return true;
            case VirtualKey.Escape:
                dismissedCompletion = Input.Text;
                ShowCompletions([]);
                return true;
        }
        return false;
    }

    // AcceptCompletion runs the chosen command, its text gone, or puts the
    // chosen file's path in place of the @ word; the agent reads the file.
    void AcceptCompletion(int index)
    {
        if (completions.ElementAtOrDefault(index) is not { Enabled: true } completion)
        {
            return;
        }
        if (!completingFiles)
        {
            var command = slashCommands.First(entry => entry.Name == completion.Id).Command;
            SetInput("");
            ShowCompletions([]);
            window.RunCommand(command);
            return;
        }
        var text = Input.Text;
        var caret = Math.Min(Input.SelectionStart, text.Length);
        var insert = completion.Id + " ";
        Input.Text = text[..completionStart] + insert + text[caret..];
        Input.SelectionStart = completionStart + insert.Length;
    }

    // ChoiceItems lists the next turn's runtimes, models, reasoning levels
    // or permission modes for the palette, as the composer's menus do.
    internal List<PaletteItem> ChoiceItems(PaletteMode mode)
    {
        var items = new List<PaletteItem>();
        void Add(string id, string title, bool chosen, Action choose, string? detail = null) =>
            items.Add(new PaletteItem(id, title, _ =>
            {
                choose();
                FocusInput();
            }, detail, Checked: chosen));
        var unchosen = Model.HasTurns(Model.Next) ? "沿用当前" : "运行时默认";
        switch (mode)
        {
            case PaletteMode.Runtime:
                foreach (var runtime in Model.Configuration.Runtimes)
                {
                    Add(runtime.Id, runtime.Name, runtime.Id == Model.Next, () => Model.ChooseRuntime(runtime.Id));
                }
                break;
            case PaletteMode.Model:
                var chosenModel = Model.Choice().Model;
                Add("", Model.HasTurns(Model.Next) && Model.NextModel() != null ? "沿用当前（" + Model.ModelName() + "）" : "运行时默认",
                    chosenModel == null, () => Model.Choose(false, choice => choice with { Model = null }));
                var index = 0;
                foreach (var model in Model.ModelChoices().OrderBy(model => model.Managed))
                {
                    Add(index++ + ":" + model.Name, model.Name, Equals(model.Selection, chosenModel),
                        () => Model.Choose(true, choice => choice with { Model = model.Selection }), model.Managed ? "Velune 提供商" : null);
                }
                break;
            case PaletteMode.Effort:
                var effort = Model.Effort();
                Add("", Model.CurrentEffort() is { } current ? "沿用当前（" + current + "）" : unchosen, effort == null,
                    () => Model.Choose(false, choice => choice with { Effort = null }));
                foreach (var level in Model.EffortOptions())
                {
                    Add(level, level, effort == level, () => Model.Choose(true, choice => choice with { Effort = level }));
                }
                break;
            case PaletteMode.Permission:
                var permission = Model.PermissionMode(Model.Choice().PermissionMode);
                var running = Model.PermissionMode(Model.CurrentSettings()?.PermissionMode);
                Add("", running != null ? "沿用当前（" + running.Label + "）" : unchosen, permission == null,
                    () => Model.Choose(false, choice => choice with { PermissionMode = null }));
                foreach (var option in Model.PermissionModes())
                {
                    Add(option.Id, option.Label, permission?.Id == option.Id, () => Model.Choose(true, choice => choice with { PermissionMode = option.Id }), option.Detail);
                }
                break;
        }
        return items;
    }

    // CardKey answers the request card: 1–9 choose an approval's option,
    // Ctrl+Backspace cancels the request, Esc goes back to the composer.
    void CardKey(object sender, KeyRoutedEventArgs e)
    {
        if (Model.Interaction is not { } interaction || Model.Busy)
        {
            return;
        }
        var control = Down(VirtualKey.Control);
        var others = Down(VirtualKey.Shift) || Down(VirtualKey.Menu);
        if (e.Key == VirtualKey.Back && control && !others && FocusManager.GetFocusedElement(XamlRoot) is not (TextBox or PasswordBox))
        {
            e.Handled = true;
            Answer(interaction.Id, new BindingRuntimeInteractionReply.Cancel());
        }
        else if (e.Key == VirtualKey.Escape && !control && !others)
        {
            e.Handled = true;
            FocusInput();
        }
        else if (interaction.Kind is BindingInteractionKind.Approval approval && !control && !others
            && Digit(e.Key) is { } digit && digit <= Math.Min(approval.Options.Length, 9))
        {
            e.Handled = true;
            Answer(interaction.Id, new BindingRuntimeInteractionReply.Decision(approval.Options[digit - 1].Id));
        }
    }

    // Answer replies to the request; the keyboard goes back to the composer
    // rather than nowhere.
    void Answer(string interactionID, BindingRuntimeInteractionReply reply)
    {
        var inCard = FocusWithin(InteractionCard);
        Model.Reply(interactionID, reply);
        if (inCard)
        {
            FocusInput();
        }
    }

    bool FocusWithin(DependencyObject root)
    {
        for (var element = XamlRoot == null ? null : FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; element != null; element = VisualTreeHelper.GetParent(element))
        {
            if (element == root)
            {
                return true;
            }
        }
        return false;
    }

    // MoveFocusOn moves the keyboard to the next field of the card.
    void MoveFocusOn()
    {
        if (XamlRoot?.Content is { } content)
        {
            FocusManager.TryMoveFocus(FocusNavigationDirection.Next, new FindNextElementOptions { SearchRoot = content });
        }
    }

    internal void FocusInteraction() => firstAnswer?.Focus(FocusState.Keyboard);

    // FocusInteractionWhenShown puts the keyboard on the request now, or
    // when the conversation shows it within a few seconds.
    internal void FocusInteractionWhenShown()
    {
        if (Model.Interaction != null && firstAnswer != null)
        {
            FocusInteraction();
        }
        else
        {
            interactionFocusRequested = DateTime.UtcNow;
        }
    }

    void TakeInteractionFocusRequest()
    {
        if (interactionFocusRequested is { } at)
        {
            interactionFocusRequested = null;
            if (DateTime.UtcNow - at < TimeSpan.FromSeconds(5))
            {
                DispatcherQueue.TryEnqueue(FocusInteraction);
            }
        }
    }

    // FocusTranscript puts the keyboard on the last row of the transcript.
    internal void FocusTranscript()
    {
        if (items.Count > 0)
        {
            FocusRow(items.Count - 1);
        }
    }

    void FocusRow(int index)
    {
        Transcript.ScrollIntoView(items[index]);
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            if (index < items.Count && Transcript.ContainerFromIndex(index) is Control row)
            {
                row.Focus(FocusState.Keyboard);
            }
        });
    }

    // TranscriptKey adds the rows' own keys to the list's arrows, Home, End
    // and paging: Enter or Space folds a turn's work, Alt+↑↓ go between the
    // user's messages, Ctrl+C copies a message (Ctrl+Shift+C its last code
    // block) and E edits a sent one.
    void TranscriptKey(object sender, KeyRoutedEventArgs e)
    {
        if (e.OriginalSource is not ListViewItem { Content: UIElement row })
        {
            return;
        }
        var index = items.IndexOf(row);
        if (index < 0)
        {
            return;
        }
        var control = Down(VirtualKey.Control);
        var shift = Down(VirtualKey.Shift);
        var alt = Down(VirtualKey.Menu);
        var plain = !control && !shift && !alt;
        rowMessages.TryGetValue(row, out var message);
        switch (e.Key)
        {
            case VirtualKey.Enter or VirtualKey.Space when plain && workRows.Values.FirstOrDefault(work => work.Element == row) is { } work:
                e.Handled = true;
                work.Toggle();
                break;
            case VirtualKey.Up or VirtualKey.Down when alt && !control && !shift:
                e.Handled = true;
                var step = e.Key == VirtualKey.Up ? -1 : 1;
                for (var i = index + step; i >= 0 && i < items.Count; i += step)
                {
                    if (rowMessages.TryGetValue(items[i], out var other) && other.Role == BindingMessageRole.User)
                    {
                        FocusRow(i);
                        break;
                    }
                }
                break;
            case VirtualKey.C when control && !alt && message != null:
                e.Handled = true;
                var text = MessageText(message);
                if (shift)
                {
                    var blocks = Regex.Matches(text, "```[^\\n]*\\n([\\s\\S]*?)```");
                    if (blocks.Count == 0)
                    {
                        break;
                    }
                    text = blocks[^1].Groups[1].Value;
                }
                var data = new DataPackage();
                data.SetText(text);
                Clipboard.SetContent(data);
                break;
            case VirtualKey.E when plain && message is { Role: BindingMessageRole.User } && Model.Editable(message.Id) != null && !Model.Busy && !Model.Running:
                e.Handled = true;
                EditMessage(message);
                break;
        }
    }
}

// The files under a working directory for @, read at most once a minute:
// git's own list inside a work tree, else a walk that skips hidden and
// build folders.
static class WorkingFiles
{
    const int Limit = 50_000;
    static readonly string[] skipped = ["node_modules", "bin", "obj", "target", "build", "dist"];
    static readonly Dictionary<string, (DateTime Read, List<string>? Files, bool Reading)> cache = [];

    // Get gives the files read, or null while the first read runs; ready
    // runs on the UI thread once a read ends.
    public static List<string>? Get(string directory, DispatcherQueue dispatcher, Action ready)
    {
        cache.TryGetValue(directory, out var known);
        if (known.Reading || (known.Files != null && DateTime.UtcNow - known.Read < TimeSpan.FromSeconds(60)))
        {
            return known.Files;
        }
        cache[directory] = (known.Read, known.Files, true);
        Task.Run(() => Read(directory)).ContinueWith(task =>
        {
            var files = task.IsCompletedSuccessfully ? task.Result : new List<string>();
            dispatcher.TryEnqueue(() =>
            {
                cache[directory] = (DateTime.UtcNow, files, false);
                ready();
            });
        }, TaskScheduler.Default);
        return known.Files;
    }

    // Rank gives the best 50 paths, the file's name weighing more than its
    // folders; shorter first among equals.
    public static IEnumerable<string> Rank(List<string> files, string query)
    {
        if (query.Length == 0)
        {
            return files.Take(50);
        }
        return files
            .Select(path => (Path: path, Score: FuzzyMatch.Score(query, [Path.GetFileName(path), path])))
            .Where(entry => entry.Score != null)
            .OrderByDescending(entry => entry.Score).ThenBy(entry => entry.Path.Length)
            .Take(50)
            .Select(entry => entry.Path);
    }

    static List<string> Read(string directory)
    {
        if (InWorkTree(directory))
        {
            try
            {
                var start = new ProcessStartInfo("git") { WorkingDirectory = directory, RedirectStandardOutput = true, UseShellExecute = false, CreateNoWindow = true };
                foreach (var argument in new[] { "ls-files", "-co", "--exclude-standard", "-z" })
                {
                    start.ArgumentList.Add(argument);
                }
                using var process = Process.Start(start);
                if (process != null)
                {
                    var output = process.StandardOutput.ReadToEnd();
                    process.WaitForExit();
                    if (process.ExitCode == 0)
                    {
                        return [.. output.Split('\0', StringSplitOptions.RemoveEmptyEntries).Take(Limit)];
                    }
                }
            }
            catch (Exception failure) when (failure is Win32Exception or IOException or InvalidOperationException)
            {
            }
        }
        var files = new List<string>();
        var folders = new Stack<string>([directory]);
        while (folders.Count > 0 && files.Count < Limit)
        {
            var folder = folders.Pop();
            try
            {
                foreach (var entry in new DirectoryInfo(folder).EnumerateFileSystemInfos())
                {
                    if (entry.Name.StartsWith('.') || entry.Attributes.HasFlag(FileAttributes.Hidden))
                    {
                        continue;
                    }
                    if (entry is DirectoryInfo)
                    {
                        if (!skipped.Contains(entry.Name, StringComparer.OrdinalIgnoreCase) && !entry.Attributes.HasFlag(FileAttributes.ReparsePoint))
                        {
                            folders.Push(entry.FullName);
                        }
                    }
                    else
                    {
                        files.Add(Path.GetRelativePath(directory, entry.FullName));
                    }
                }
            }
            catch (Exception failure) when (failure is IOException or UnauthorizedAccessException or System.Security.SecurityException)
            {
            }
        }
        files.Sort(StringComparer.OrdinalIgnoreCase);
        return files;
    }

    static bool InWorkTree(string directory)
    {
        for (var folder = new DirectoryInfo(directory); folder != null; folder = folder.Parent)
        {
            if (Path.Exists(Path.Combine(folder.FullName, ".git")))
            {
                return true;
            }
        }
        return false;
    }
}
