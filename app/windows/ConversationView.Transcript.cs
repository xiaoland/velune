using System.Diagnostics;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media.Imaging;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

// The transcript shows the core's display items in order: each turn is the
// user's message, its work process folded under one row, and the reply.
public sealed partial class ConversationView
{
    readonly System.Collections.ObjectModel.ObservableCollection<UIElement> items = [];
    // Rows by display item ID, rebuilt when their content changes. Each
    // element lives in the list only, and a work row's members in that row.
    readonly Dictionary<string, (string Version, UIElement Element)> rows = [];
    readonly Dictionary<string, WorkRow> workRows = [];
    // Which reasoning and tool outputs the user unfolded, by block.
    readonly HashSet<string> expanded = [];
    // The user's folding of a turn's work, kept while the turn stays as it
    // was: a running turn shows its work, a finished one folds it again.
    readonly Dictionary<string, (bool Running, bool Expanded)> unfolded = [];
    ScrollViewer? scroller;
    // Following keeps the newest content in view until the user scrolls away.
    bool following = true;
    bool scrolled;
    // The outline folded in place: the list shows the user's messages only.
    bool outlineFolded;

    void AttachTranscript()
    {
        Transcript.ItemsSource = items;
        Transcript.Loaded += (_, _) =>
        {
            if (scroller != null)
            {
                return;
            }
            scroller = Ui.Descendant<ScrollViewer>(Transcript);
            if (scroller != null)
            {
                scroller.ViewChanged += (_, e) =>
                {
                    if (!e.IsIntermediate)
                    {
                        Follow(AtBottom());
                    }
                };
            }
        };
        BottomButton.Click += (_, _) => ScrollToEnd();
        OutlineButton.Click += (_, _) => OutlineClicked();
    }

    bool AtBottom() => scroller == null || scroller.VerticalOffset >= scroller.ScrollableHeight - 8;

    void Follow(bool follow)
    {
        following = follow;
        if (Transcript.ItemsPanelRoot is ItemsStackPanel panel)
        {
            panel.ItemsUpdatingScrollMode = follow ? ItemsUpdatingScrollMode.KeepLastItemInView : ItemsUpdatingScrollMode.KeepItemsInView;
        }
        BottomButton.Visibility = Visible(!follow && items.Count > 0);
    }

    void ScrollToEnd()
    {
        Follow(true);
        if (items.Count == 0)
        {
            return;
        }
        Transcript.ScrollIntoView(items[^1], ScrollIntoViewAlignment.Leading);
        // After layout, so the last row's measured height counts.
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            if (IsLoaded && following && scroller != null)
            {
                scroller.ChangeView(null, scroller.ScrollableHeight, null, true);
            }
        });
    }

    void RenderTranscript()
    {
        var wanted = new List<UIElement>();
        var shown = new HashSet<string>();
        // Add keeps a row, rebuilt when its version changes; a folded outline
        // keeps the others built but out of the list.
        void Add(string key, string version, Func<UIElement> build, bool listed = true)
        {
            shown.Add(key);
            if (!rows.TryGetValue(key, out var row) || row.Version != version)
            {
                row = (version, build());
                rows[key] = row;
            }
            if (listed)
            {
                wanted.Add(row.Element);
            }
        }
        if (Model.Snapshot is { } snapshot)
        {
            var messages = Messages();
            var turns = snapshot.TranscriptTurns.ToDictionary(turn => turn.Id);
            var turnOfUser = snapshot.TranscriptTurns.ToDictionary(turn => turn.UserMessageId);
            var replies = snapshot.TranscriptTurns
                .Where(turn => turn.ReplyStartBlock > 0 && turn.LastMessageId != null)
                .ToDictionary(turn => turn.LastMessageId!, turn => (int)turn.ReplyStartBlock);
            foreach (var item in snapshot.TranscriptItems)
            {
                switch (item)
                {
                    case BindingTranscriptItem.Message entry when messages.TryGetValue(entry.MessageId, out var message):
                        // A reply sharing its message with work starts at the reply's first block.
                        var from = entry.Id.StartsWith("reply:", StringComparison.Ordinal) && replies.TryGetValue(message.Id, out var start) ? start : 0;
                        var user = message.Role == BindingMessageRole.User;
                        Add(entry.Id, Version(message, from), () => Row(message, from, entry.Id), !outlineFolded || user);
                        // A turn without work still shows what it ran with.
                        if (user && turnOfUser.TryGetValue(message.Id, out var turn) && turn.WorkMessageIds.Length == 0
                            && (turn.Settings != null || turn.Usage != null || turn.DurationMs != null))
                        {
                            Add("turn:" + turn.Id, TurnVersion(turn), () => TurnRow(turn), !outlineFolded);
                        }
                        break;
                    case BindingTranscriptItem.Work work when turns.TryGetValue(work.TurnId, out var worked):
                        shown.Add("work:" + worked.Id);
                        if (!workRows.TryGetValue(worked.Id, out var workRow))
                        {
                            workRow = new WorkRow(this);
                            workRows[worked.Id] = workRow;
                        }
                        workRow.Update(worked, messages);
                        if (!outlineFolded)
                        {
                            wanted.Add(workRow.Element);
                        }
                        break;
                }
            }
        }
        if (Model.Sending is { } sending)
        {
            Add("sending", (Model.Accepted ? "1" : "0") + sending, () => SendingRow(sending, Model.Accepted));
        }
        if (Model.Working && !outlineFolded)
        {
            var status = Status();
            Add("status:" + status, "", () => StatusRow(status));
        }
        foreach (var gone in rows.Keys.Where(key => !shown.Contains(key)).ToList())
        {
            rows.Remove(gone);
        }
        foreach (var gone in workRows.Keys.Where(id => !shown.Contains("work:" + id)).ToList())
        {
            workRows.Remove(gone);
        }
        Ui.Reconcile(items, wanted);
        if (!scrolled && items.Count > 0)
        {
            scrolled = true;
            ScrollToEnd();
        }
        else if (following && !outlineFolded)
        {
            ScrollToEnd();
        }
        else
        {
            BottomButton.Visibility = Visible(!following && items.Count > 0);
        }
    }

    // Messages are the transcript's messages by ID, with any content read in full.
    Dictionary<string, BindingMessage> Messages()
    {
        var messages = new Dictionary<string, BindingMessage>(Model.Messages.Count);
        foreach (var message in Model.Messages)
        {
            messages[message.Id] = Model.Payloads.TryGetValue(message.Id, out var loaded) ? loaded : message;
        }
        return messages;
    }

    string Version(BindingMessage message, int from) =>
        $"{message.Stamp}/{Model.Payloads.ContainsKey(message.Id)}/{from}";

    static string TurnVersion(BindingTranscriptTurn turn) =>
        $"{turn.DurationMs}/{turn.IsRunning}/{turn.Settings}/{turn.Usage}";

    UIElement Row(BindingMessage message, int from, string id)
    {
        var element = Message(message, from, message.Blocks.Length);
        if (message.Role == BindingMessageRole.User)
        {
            // In the folded outline, a message unfolds the list at itself.
            element.Tapped += (_, _) =>
            {
                if (outlineFolded)
                {
                    SetOutlineFolded(false, id);
                }
            };
        }
        return element;
    }

    // Message lays out the user's messages on the right, the model's on the
    // left, and the system's and the Harness's in the middle, from block
    // `from` up to `to`.
    UIElement Message(BindingMessage message, int from, int to)
    {
        if (message.Role == BindingMessageRole.User)
        {
            return UserMessage(message);
        }
        var blocks = new StackPanel { Spacing = message.Role == BindingMessageRole.Assistant ? 8 : 4 };
        for (var position = from; position < to; position++)
        {
            if (Block(message, position, message.Blocks[position]) is { } block)
            {
                blocks.Children.Add(block);
            }
        }
        if (message.Role == BindingMessageRole.System)
        {
            // The system's and the Harness's states sit in the middle of the list.
            blocks.HorizontalAlignment = HorizontalAlignment.Stretch;
            return blocks;
        }
        return Column(blocks);
    }

    // Column holds content in the content column: as wide as the list
    // allows up to a readable width, aligned to its left edge.
    static Grid Column(UIElement content)
    {
        var column = new Grid();
        column.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star), MaxWidth = 820 });
        column.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(0, GridUnitType.Auto) });
        column.Children.Add(content);
        return column;
    }

    // UserMessage is a bubble on the right, its attachments above it:
    // images first, then files.
    UIElement UserMessage(BindingMessage message)
    {
        var panel = new StackPanel { Spacing = 6, HorizontalAlignment = HorizontalAlignment.Right, MaxWidth = 560 };
        var attached = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, HorizontalAlignment = HorizontalAlignment.Right };
        var files = new List<UIElement>();
        var bubble = new StackPanel { Spacing = 8 };
        for (var position = 0; position < message.Blocks.Length; position++)
        {
            switch (message.Blocks[position])
            {
                case BindingMessageBlock.Image:
                    attached.Children.Add(Thumbnail(message.Id, position));
                    break;
                case BindingMessageBlock.File file:
                    files.Add(FileChip(file.Path));
                    break;
                case BindingMessageBlock.AgentSender sender:
                    var source = new HyperlinkButton { Content = $"来自 {sender.RuntimeName} 会话「{sender.Title}」", Padding = new Thickness(0), FontSize = 12 };
                    source.Click += (_, _) => window.OpenLinked(sender.ConversationId);
                    bubble.Children.Insert(0, source);
                    break;
                case BindingMessageBlock.Text text:
                    bubble.Children.Add(new TextBlock { Text = text.TextValue, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap });
                    break;
                case BindingMessageBlock.Notice notice:
                    bubble.Children.Add(Muted(notice.TextValue));
                    break;
            }
        }
        foreach (var file in files)
        {
            attached.Children.Add(file);
        }
        if (attached.Children.Count > 0)
        {
            panel.Children.Add(attached);
        }
        if (bubble.Children.Count > 0)
        {
            panel.Children.Add(new Border
            {
                HorizontalAlignment = HorizontalAlignment.Right,
                Padding = new Thickness(12, 8, 12, 8),
                CornerRadius = new CornerRadius(8),
                Style = Ui.Style("CardStyle"),
                Child = bubble,
            });
        }
        return panel;
    }

    // Block draws one block; reasoning without a summary has nothing to show.
    UIElement? Block(BindingMessage message, int position, BindingMessageBlock block)
    {
        var key = $"{message.Id}/{position}";
        switch (block)
        {
            case BindingMessageBlock.Text text:
                return Markdown.View(text.TextValue, OpenLink);
            case BindingMessageBlock.Notice notice:
                var centered = Muted(notice.TextValue);
                centered.HorizontalAlignment = HorizontalAlignment.Center;
                centered.TextAlignment = TextAlignment.Center;
                return centered;
            case BindingMessageBlock.Reasoning reasoning:
                var summary = reasoning.Summary.Trim();
                if (summary.Length == 0)
                {
                    return null;
                }
                var empty = reasoning.TextValue is BindingMessagePayload.Inline { Text: var thought } && thought.Trim().Length == 0;
                return empty ? Muted(summary) : Disclosure(message.Id, key, summary, reasoning.TextValue);
            case BindingMessageBlock.Tool tool:
                var suffix = tool.State switch
                {
                    BindingToolState.Pending => " · 等待",
                    BindingToolState.Running => " · 运行中",
                    BindingToolState.Failed => " · 失败",
                    _ => "",
                };
                var disclosure = Disclosure(message.Id, key, tool.Title + suffix, tool.Output);
                if (tool.LinkedConversationId is not { } linked)
                {
                    return disclosure;
                }
                // A collaboration tool or a native sub-agent opens its conversation.
                var open = new HyperlinkButton { Content = "打开会话", Padding = new Thickness(0), FontSize = 13 };
                open.Click += (_, _) => window.OpenLinked(linked);
                return new StackPanel { Spacing = 2, Children = { disclosure, open } };
            case BindingMessageBlock.Image:
                return Thumbnail(message.Id, position);
            case BindingMessageBlock.File file:
                return FileChip(file.Path);
            case BindingMessageBlock.AgentSender sender:
                return Muted($"来自 {sender.RuntimeName} 会话「{sender.Title}」");
            default:
                return null;
        }
    }

    static TextBlock Muted(string text) => new()
    {
        Text = text,
        FontSize = 13,
        IsTextSelectionEnabled = true,
        TextWrapping = TextWrapping.Wrap,
        Style = Ui.Style("SecondaryTextStyle"),
    };

    // Disclosure keeps reasoning and tool output folded; large ones are read
    // when unfolded.
    UIElement Disclosure(string messageID, string key, string title, BindingMessagePayload? payload)
    {
        if (payload == null)
        {
            return Muted(title);
        }
        var expander = new Expander
        {
            Header = new TextBlock { Text = title, FontSize = 13, TextTrimming = TextTrimming.CharacterEllipsis },
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            IsExpanded = expanded.Contains(key),
            Content = payload switch
            {
                BindingMessagePayload.Inline inline => Markdown.Code(inline.Text, 12),
                _ => new ProgressRing { IsActive = true, Width = 20, Height = 20 },
            },
        };
        expander.Expanding += (_, _) =>
        {
            expanded.Add(key);
            if (payload is BindingMessagePayload.Deferred)
            {
                Model.LoadPayload(messageID);
            }
        };
        expander.Collapsed += (_, _) => expanded.Remove(key);
        return expander;
    }

    // Thumbnail shows an attached image, read when shown; it opens in the
    // system's default app.
    UIElement Thumbnail(string messageID, int block)
    {
        var image = new Image { Height = 72, MaxWidth = 160, Stretch = Microsoft.UI.Xaml.Media.Stretch.Uniform };
        var frame = new Button
        {
            Padding = new Thickness(2),
            MinWidth = 72,
            MinHeight = 72,
            Content = new ProgressRing { IsActive = true, Width = 20, Height = 20 },
        };
        ToolTipService.SetToolTip(frame, "图片");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(frame, "图片");
        BindingAttachmentContent? content = null;
        frame.Loaded += async (_, _) =>
        {
            if (content != null)
            {
                return;
            }
            try
            {
                content = await Model.ReadAttachment(messageID, block);
                var bitmap = new BitmapImage { DecodePixelHeight = 144 };
                using var stream = new MemoryStream(content.Bytes).AsRandomAccessStream();
                await bitmap.SetSourceAsync(stream);
                image.Source = bitmap;
                frame.Content = image;
            }
            catch (Exception failure)
            {
                frame.Content = Muted("［图片］");
                ToolTipService.SetToolTip(frame, Core.Describe(failure));
            }
        };
        frame.Click += (_, _) =>
        {
            if (content == null)
            {
                return;
            }
            var extension = content.MediaType switch
            {
                "image/jpeg" => ".jpg",
                "image/gif" => ".gif",
                "image/webp" => ".webp",
                _ => ".png",
            };
            var folder = Path.Combine(Path.GetTempPath(), "Velune", "attachments");
            Directory.CreateDirectory(folder);
            var path = Path.Combine(folder, $"{messageID.GetHashCode():x8}-{block}{extension}");
            File.WriteAllBytes(path, content.Bytes);
            Launch(path);
        };
        return frame;
    }

    // FileChip names an attached file; it opens in its default app.
    UIElement FileChip(string path)
    {
        var chip = new Button
        {
            Content = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 6,
                Children =
                {
                    new FontIcon { Glyph = "\uE8A5", FontSize = 14 },
                    new TextBlock { Text = Listing.LastPart(path), TextTrimming = TextTrimming.CharacterEllipsis, MaxWidth = 220 },
                },
            },
        };
        ToolTipService.SetToolTip(chip, path);
        chip.Click += (_, _) => OpenPath(path, path);
        return chip;
    }

    // OpenLink opens a message's link: web and mail links in their apps,
    // files relative to the conversation's directory.
    void OpenLink(string link)
    {
        if (Uri.TryCreate(link, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https" or "mailto")
        {
            Launch(uri.AbsoluteUri, link);
            return;
        }
        var path = Uri.TryCreate(link, UriKind.Absolute, out var file) && file.IsFile ? file.LocalPath : Uri.UnescapeDataString(link);
        // A line or anchor suffix names a place in the file, not part of it.
        var hash = path.IndexOf('#');
        if (hash > 0)
        {
            path = path[..hash];
        }
        if (!Path.IsPathFullyQualified(path) && Model.Snapshot?.Conversation.Cwd is { } cwd)
        {
            path = Path.Combine(cwd, path);
        }
        OpenPath(path, link);
    }

    // OpenPath opens a file on this PC; one on another device is only named.
    void OpenPath(string path, string link)
    {
        if (!Model.Device.IsLocal)
        {
            CantOpen($"这个文件在 {window.Hub.DeviceName(Model.Device)} 上", link);
            return;
        }
        path = Path.GetFullPath(path.Replace('/', '\\'));
        if (!File.Exists(path) && !Directory.Exists(path))
        {
            CantOpen("找不到这个文件", link);
            return;
        }
        Launch(path, link);
    }

    void Launch(string target, string? link = null)
    {
        try
        {
            Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
        }
        catch (Exception failure)
        {
            CantOpen("无法打开：" + failure.Message, link ?? target);
        }
    }

    // CantOpen shows the link as written, to copy.
    async void CantOpen(string title, string link)
    {
        var text = new TextBox { Text = link, IsReadOnly = true, TextWrapping = TextWrapping.Wrap };
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = title,
            Content = text,
            PrimaryButtonText = "复制",
            CloseButtonText = "关闭",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            var package = new DataPackage();
            package.SetText(link);
            Clipboard.SetContent(package);
        }
    }

    // SendingRow is the message just sent, marked until the runtime takes it.
    static UIElement SendingRow(string text, bool accepted)
    {
        var panel = new StackPanel { Spacing = 4, HorizontalAlignment = HorizontalAlignment.Right, MaxWidth = 560 };
        panel.Children.Add(new Border
        {
            HorizontalAlignment = HorizontalAlignment.Right,
            Padding = new Thickness(12, 8, 12, 8),
            CornerRadius = new CornerRadius(8),
            Style = Ui.Style("CardStyle"),
            Child = new TextBlock { Text = text, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap },
        });
        if (!accepted)
        {
            var mark = Muted("发送中");
            mark.HorizontalAlignment = HorizontalAlignment.Right;
            panel.Children.Add(mark);
        }
        return panel;
    }

    // Status is what the running turn is doing: the latest of its reasoning
    // summary, running tool or reply.
    string Status()
    {
        var turn = Model.Snapshot?.TranscriptTurns.LastOrDefault(turn => turn.IsRunning);
        if (turn == null)
        {
            return "正在运行";
        }
        var messages = Messages();
        var ids = turn.WorkMessageIds.ToList();
        if (turn.LastMessageId is { } last && !ids.Contains(last))
        {
            ids.Add(last);
        }
        foreach (var id in Enumerable.Reverse(ids))
        {
            if (!messages.TryGetValue(id, out var message))
            {
                continue;
            }
            foreach (var block in Enumerable.Reverse(message.Blocks))
            {
                switch (block)
                {
                    case BindingMessageBlock.Tool { State: BindingToolState.Running or BindingToolState.Pending } tool:
                        return tool.Title;
                    case BindingMessageBlock.Reasoning { Summary: var summary } when summary.Trim().Length > 0:
                        return summary.Trim();
                    case BindingMessageBlock.Text:
                        return "正在回复";
                }
            }
        }
        return "正在运行";
    }

    static UIElement StatusRow(string status) => Column(new StackPanel
    {
        Orientation = Orientation.Horizontal,
        Spacing = 8,
        Children =
        {
            new ProgressRing { IsActive = true, Width = 16, Height = 16 },
            new TextBlock { Text = status, Style = Ui.Style("SecondaryTextStyle"), TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center },
        },
    });

    // TurnRow is the row of a turn without work: what it ran with, and its
    // time when observed.
    UIElement TurnRow(BindingTranscriptTurn turn)
    {
        var info = new TurnInfo(this, turn);
        var row = new Grid { MinHeight = 32 };
        if (turn.DurationMs is { } ms)
        {
            row.Children.Add(new TextBlock { Text = "用时 " + Duration(ms), VerticalAlignment = VerticalAlignment.Center });
        }
        info.Panel.HorizontalAlignment = HorizontalAlignment.Right;
        row.Children.Add(info.Panel);
        var card = new Border { Padding = new Thickness(16, 0, 8, 0), CornerRadius = new CornerRadius(4), Style = Ui.Style("CardStyle"), Child = row };
        info.Hover(card);
        return Column(card);
    }

    static string Duration(ulong ms)
    {
        var seconds = (long)(ms / 1000);
        return seconds switch
        {
            < 60 => $"{seconds} 秒",
            < 3600 => $"{seconds / 60} 分 {seconds % 60} 秒",
            _ => $"{seconds / 3600} 小时 {seconds % 3600 / 60} 分",
        };
    }

    // A turn's work process: one folded row whose members are the turn's
    // work messages, built while it is unfolded.
    sealed class WorkRow
    {
        readonly ConversationView view;
        readonly Expander expander;
        readonly TextBlock label = new() { VerticalAlignment = VerticalAlignment.Center };
        readonly ProgressRing progress = new() { Width = 16, Height = 16 };
        readonly StackPanel members = new() { Spacing = 8 };
        readonly Dictionary<string, (string Version, UIElement Element)> rendered = [];
        readonly TurnInfo info;
        BindingTranscriptTurn turn = null!;
        Dictionary<string, BindingMessage> messages = [];
        // Set while the row itself folds or unfolds, which is not the user's choice.
        bool applying;

        public UIElement Element { get; }

        public WorkRow(ConversationView view)
        {
            this.view = view;
            info = new TurnInfo(view, null);
            expander = new Expander
            {
                Header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { progress, label } },
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Content = members,
            };
            expander.Expanding += (_, _) =>
            {
                if (!applying)
                {
                    view.unfolded[turn.Id] = (turn.IsRunning, true);
                }
                Fill();
            };
            expander.Collapsed += (_, _) =>
            {
                if (!applying)
                {
                    view.unfolded[turn.Id] = (turn.IsRunning, false);
                }
            };
            // The turn's settings sit at the header's end, clear of its chevron.
            info.Panel.HorizontalAlignment = HorizontalAlignment.Right;
            info.Panel.VerticalAlignment = VerticalAlignment.Top;
            info.Panel.Margin = new Thickness(0, 8, 48, 0);
            var row = new Grid { Children = { expander, info.Panel } };
            info.Hover(row);
            Element = Column(row);
        }

        public void Update(BindingTranscriptTurn turn, Dictionary<string, BindingMessage> messages)
        {
            this.turn = turn;
            this.messages = messages;
            label.Text = turn.DurationMs is { } ms ? "用时 " + Duration(ms) : "工作过程";
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(expander, label.Text);
            progress.IsActive = turn.IsRunning;
            progress.Visibility = Visible(turn.IsRunning);
            info.Update(turn);
            var open = view.unfolded.TryGetValue(turn.Id, out var choice) && choice.Running == turn.IsRunning ? choice.Expanded : turn.IsRunning;
            if (expander.IsExpanded != open)
            {
                applying = true;
                expander.IsExpanded = open;
                applying = false;
            }
            if (open)
            {
                Fill();
            }
        }

        // Fill builds the members shown: each work message, and the work
        // blocks of a reply's message before its reply starts.
        void Fill()
        {
            var wanted = new List<UIElement>();
            var ids = new HashSet<string>();
            foreach (var id in turn.WorkMessageIds)
            {
                if (!messages.TryGetValue(id, out var message))
                {
                    continue;
                }
                var to = id == turn.LastMessageId ? (int)turn.ReplyStartBlock : message.Blocks.Length;
                var version = view.Version(message, 0) + "/" + to;
                ids.Add(id);
                if (!rendered.TryGetValue(id, out var known) || known.Version != version)
                {
                    known = (version, view.Message(message, 0, to));
                    rendered[id] = known;
                }
                wanted.Add(known.Element);
            }
            foreach (var gone in rendered.Keys.Where(id => !ids.Contains(id)).ToList())
            {
                rendered.Remove(gone);
            }
            Ui.Reconcile(members.Children, wanted);
        }
    }

    // What a turn ran with: a plain icon, with the runtime and model beside
    // it while the pointer is on the row, and all of it on click.
    sealed class TurnInfo
    {
        readonly ConversationView view;
        readonly TextBlock summary = new() { VerticalAlignment = VerticalAlignment.Center, Visibility = Visibility.Collapsed };
        readonly Button button;
        BindingTranscriptTurn? turn;

        public StackPanel Panel { get; }

        public TurnInfo(ConversationView view, BindingTranscriptTurn? turn)
        {
            this.view = view;
            summary.Style = Ui.Style("SecondaryTextStyle");
            summary.FontSize = 12;
            button = new Button
            {
                Padding = new Thickness(6),
                Background = null,
                BorderThickness = new Thickness(0),
                Content = new FontIcon { Glyph = "\uE946", FontSize = 14 },
            };
            ToolTipService.SetToolTip(button, "本轮配置");
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, "本轮配置");
            var flyout = new Flyout();
            flyout.Opening += (_, _) => flyout.Content = this.turn is { } shown ? view.TurnDetails(shown) : null;
            button.Flyout = flyout;
            Panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, Children = { summary, button } };
            if (turn != null)
            {
                Update(turn);
            }
        }

        public void Update(BindingTranscriptTurn turn)
        {
            this.turn = turn;
            var settings = turn.Settings;
            var parts = new[] { view.RuntimeName(settings?.RuntimeId), settings?.Model }.Where(part => !string.IsNullOrEmpty(part));
            summary.Text = string.Join(" · ", parts);
            Panel.Visibility = Visible(settings != null || turn.Usage != null);
        }

        public void Hover(UIElement row)
        {
            row.PointerEntered += (_, _) => summary.Visibility = Visible(summary.Text.Length > 0);
            row.PointerExited += (_, _) => summary.Visibility = Visibility.Collapsed;
        }
    }

    string? RuntimeName(string? id) =>
        id == null ? null : Model.Configuration.Runtime(id)?.Name ?? id;

    // TurnDetails lists what a turn ran with and used, as recorded; what no
    // record says is left out, and token counts not recorded are marked so.
    UIElement TurnDetails(BindingTranscriptTurn turn)
    {
        var grid = new Grid { ColumnSpacing = 16, RowSpacing = 6, MinWidth = 260 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        void Line(string name, string? value)
        {
            if (string.IsNullOrEmpty(value))
            {
                return;
            }
            var row = grid.RowDefinitions.Count;
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var label = new TextBlock { Text = name, Style = Ui.Style("SecondaryTextStyle") };
            var text = new TextBlock { Text = value, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
            Grid.SetRow(label, row);
            Grid.SetRow(text, row);
            Grid.SetColumn(text, 1);
            grid.Children.Add(label);
            grid.Children.Add(text);
        }
        var settings = turn.Settings;
        var runtimeID = settings?.RuntimeId ?? Model.Snapshot?.ContextRuntimeId;
        Line("运行时", RuntimeName(settings?.RuntimeId));
        Line("模型", settings?.Model);
        Line("推理强度", settings?.Effort);
        if (settings?.PermissionMode is { } mode)
        {
            var known = runtimeID == null ? null : Model.Configuration.RuntimeType(runtimeID)?.PermissionModes.FirstOrDefault(candidate => candidate.Id == mode);
            Line("权限", known?.Label ?? mode);
        }
        if (turn.Usage is { } usage)
        {
            static string Count(ulong? tokens) => tokens is { } count ? count.ToString("N0") : "未记录";
            Line("输入 token", Count(usage.InputTokens));
            Line("缓存读取", Count(usage.CacheReadTokens));
            Line("缓存写入", Count(usage.CacheWriteTokens));
            Line("输出 token", Count(usage.OutputTokens));
            Line("推理 token", usage.ReasoningTokens is { } reasoning ? reasoning.ToString("N0") : "未单独记录");
            if (usage.ContextTokens is { } context)
            {
                Line("结束时上下文", usage.ContextWindow is { } window && window > 0
                    ? $"{context:N0} / {window:N0}（{context * 100.0 / window:0}%）"
                    : $"{context:N0}");
            }
        }
        if (turn.DurationMs is { } ms)
        {
            Line("用时", Duration(ms));
        }
        return new StackPanel
        {
            Spacing = 12,
            Children = { new TextBlock { Text = "本轮配置", Style = Ui.Style("BodyStrongTextBlockStyle") }, grid },
        };
    }

    void OutlineClicked()
    {
        if (Model.Configuration.Outline == BindingOutlinePresentation.InPlace)
        {
            SetOutlineFolded(OutlineButton.IsChecked == true, null);
            return;
        }
        OutlineButton.IsChecked = false;
        ShowOutline();
    }

    // SetOutlineFolded folds the list to the user's messages, or back to all
    // of it at a message, else where the reader was.
    void SetOutlineFolded(bool folded, string? at)
    {
        var anchor = at ?? VisibleUserMessage();
        outlineFolded = folded;
        OutlineButton.IsChecked = folded;
        RenderTranscript();
        if (anchor != null)
        {
            GoTo(anchor);
        }
    }

    // VisibleUserMessage is the user's message at or above the top of the view.
    string? VisibleUserMessage()
    {
        if (scroller == null || Model.Snapshot is not { } snapshot)
        {
            return null;
        }
        string? found = null;
        foreach (var entry in snapshot.TranscriptOutline)
        {
            if (!rows.TryGetValue(entry.Id, out var row) || row.Element is not FrameworkElement { IsLoaded: true } element || !items.Contains(element))
            {
                continue;
            }
            var top = element.TransformToVisual(scroller).TransformPoint(new Windows.Foundation.Point(0, 0)).Y;
            if (top > scroller.ViewportHeight / 2)
            {
                break;
            }
            found = entry.Id;
        }
        return found;
    }

    // GoTo brings a message to the top of the view, again after layout, so
    // estimated heights do not leave it at a neighbour.
    void GoTo(string id)
    {
        if (!rows.TryGetValue(id, out var row) || !items.Contains(row.Element))
        {
            return;
        }
        Follow(false);
        Transcript.ScrollIntoView(row.Element, ScrollIntoViewAlignment.Leading);
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            if (IsLoaded && items.Contains(row.Element))
            {
                Transcript.ScrollIntoView(row.Element, ScrollIntoViewAlignment.Leading);
            }
        });
    }

    // ShowOutline lists the user's messages in a flyout at the button: one
    // click chooses, a double click or 前往 goes there.
    void ShowOutline()
    {
        var entries = Model.Snapshot?.TranscriptOutline ?? [];
        var messages = Messages();
        var list = new ListView { SelectionMode = ListViewSelectionMode.Single, MaxHeight = 420, Width = 360 };
        foreach (var entry in entries)
        {
            var text = messages.TryGetValue(entry.MessageId, out var message) ? FirstLine(message) : "";
            list.Items.Add(new ListViewItem
            {
                Tag = entry.Id,
                Content = new TextBlock { Text = text, TextTrimming = TextTrimming.CharacterEllipsis },
            });
        }
        var go = new Button { Content = "前往", IsEnabled = false, HorizontalAlignment = HorizontalAlignment.Right };
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(entries.Length == 0 ? Muted("还没有用户消息。") : list);
        panel.Children.Add(go);
        var flyout = new Flyout { Content = panel, Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        void Go()
        {
            if (list.SelectedItem is not ListViewItem { Tag: string id })
            {
                return;
            }
            flyout.Hide();
            // The last message while the conversation runs means its end.
            if (Model.Working && id == entries[^1].Id)
            {
                ScrollToEnd();
                return;
            }
            GoTo(id);
        }
        list.SelectionChanged += (_, _) => go.IsEnabled = list.SelectedItem != null;
        list.DoubleTapped += (_, _) => Go();
        go.Click += (_, _) => Go();
        flyout.ShowAt(OutlineButton);
        if (list.Items.Count > 0)
        {
            list.SelectedIndex = list.Items.Count - 1;
            list.ScrollIntoView(list.Items[^1]);
        }
    }

    static string FirstLine(BindingMessage message)
    {
        foreach (var block in message.Blocks)
        {
            if (block is BindingMessageBlock.Text text && text.TextValue.Trim().Length > 0)
            {
                return text.TextValue.Trim().Split('\n')[0].Trim();
            }
        }
        return message.Blocks.Any(block => block is BindingMessageBlock.Image) ? "［图片］" : "［附件］";
    }
}
