using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Velune.Bindings;

namespace Velune;

public sealed partial class WorkspaceWindow
{
    internal async void OpenAgentUri(string uri, Device device)
    {
        if (closed) { return; }
        if (!Root.IsLoaded)
        {
            void OpenWhenLoaded(object sender, RoutedEventArgs args)
            {
                Root.Loaded -= OpenWhenLoaded;
                OpenAgentUri(uri, device);
            }
            Root.Loaded += OpenWhenLoaded;
            return;
        }
        try
        {
            var result = await Shell.Core.Call(device, NativeOperation.ConversationRead, application => application.ResolveAgentUri(uri, null));
            if (closed) { return; }
            foreach (var problem in result.Failures) { Problems.Report("打开会话 · " + problem.RuntimeId, problem.Detail, device); }
            BindingConversationSummary? chosen = null;
            if (result.Conversations.Length == 1 && result.Failures.Length == 0)
            {
                chosen = result.Conversations[0].Conversation;
            }
            else
            {
                var failure = Ui.Error();
                if (result.Failures.Length > 0) { Ui.Show(failure, "部分运行时无法读取，详见“问题”。"); }
                else if (result.Conversations.Length == 0) { Ui.Show(failure, "这台设备的已配置实例中找不到该会话。"); }
                var matches = new ListView { SelectionMode = ListViewSelectionMode.Single, MaxHeight = 300 };
                foreach (var item in result.Conversations)
                {
                    var summary = item.Conversation;
                    var name = Shell.Listings.Of(device).Configuration.Runtime(summary.RuntimeId)?.Name ?? summary.RuntimeId;
                    var content = new StackPanel { Spacing = 2, Children = { Ui.Text(summary.Title), Ui.Muted(name) } };
                    if (summary.Cwd is { } cwd) { content.Children.Add(Ui.Muted(cwd)); }
                    var row = new ListViewItem { Content = content, Tag = summary };
                    Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(row, summary.Title + " · " + name);
                    matches.Items.Add(row);
                }
                var dialog = new ContentDialog
                {
                    Title = "打开会话", PrimaryButtonText = matches.Items.Count > 0 ? "打开" : "", CloseButtonText = "关闭",
                    IsPrimaryButtonEnabled = false,
                    Content = new StackPanel { MinWidth = 420, Spacing = 12, Children = { failure, matches } },
                };
                matches.SelectionChanged += (_, _) => dialog.IsPrimaryButtonEnabled = matches.SelectedItem != null;
                if (await ShowDialog(dialog) == ContentDialogResult.Primary)
                {
                    chosen = (matches.SelectedItem as ListViewItem)?.Tag as BindingConversationSummary;
                }
            }
            if (!closed && chosen is { } conversation)
            {
                SwitchTo(device);
                Open(new Target(conversation.RuntimeId, conversation.Id, conversation.Title), Opening.CurrentTab);
            }
        }
        catch (Exception error)
        {
            if (!closed) { Problems.Report("打开会话", error, device); }
        }
    }
}
