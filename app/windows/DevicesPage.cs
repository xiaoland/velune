using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using QRCoder;
using Velune.Bindings;
using Windows.ApplicationModel.DataTransfer;

namespace Velune;

// This PC's side of remote control, and the devices it controls, as Mac's
// device settings are.
sealed class DevicesPage : UserControl
{
    readonly Core core;
    readonly Hub hub;
    readonly SettingsWindow window;
    // Open shows a paired device in the main window.
    readonly Action<Device> open;
    readonly StackPanel panel = new() { Spacing = 8, Padding = new Thickness(24, 12, 24, 24), MaxWidth = 1000 };
    readonly InfoBar failure = Ui.FailureBar();
    readonly TextBox invite = new() { PlaceholderText = "粘贴另一台设备的邀请" };
    readonly Button connect = new() { Content = "连接" };
    readonly Border inviteCard;
    BindingRemoteInvitation? invitation;
    BitmapImage? code;
    bool copied;
    bool busy;
    readonly DispatcherTimer expiry = new() { Interval = TimeSpan.FromSeconds(15) };

    internal DevicesPage(Core core, Hub hub, SettingsWindow window, Action<Device> open)
    {
        this.core = core;
        this.hub = hub;
        this.window = window;
        this.open = open;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(invite, "邀请");
        invite.TextChanged += (_, _) => connect.IsEnabled = Ready;
        connect.Click += (_, _) => Connect();
        invite.KeyDown += (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter && Ready)
            {
                e.Handled = true;
                Connect();
            }
        };
        inviteCard = InviteCard();
        // Only while shown: the page exists from the settings window's start.
        expiry.Tick += (_, _) => Render();
        Content = new ScrollViewer { Content = panel };
        Unloaded += (_, _) => expiry.Stop();
        Loaded += (_, _) => expiry.Start();
        Render();
    }

    // HubChanged retires a shown invitation once someone used it, or it expired.
    internal void HubChanged()
    {
        if (invitation != null && hub.State?.IncomingPairings.Any(pairing => pairing.Phase == BindingPairingPhase.AwaitingApproval) == true)
        {
            invitation = null;
        }
        Render();
    }

    bool Ready => !busy && hub.State != null && invite.Text.Trim().Length > 0;

    static bool Expired(ulong expiresAtUnixSeconds) => (ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds() > expiresAtUnixSeconds;

    // Perform runs one call on the remote service and shows its failure;
    // done runs on the UI thread after work succeeds.
    async void Perform(Action<VeluneRemoteEndpoint> work, Action? done = null)
    {
        if (busy)
        {
            return;
        }
        busy = true;
        Ui.Show(failure, "");
        Render();
        try
        {
            await Task.Run(() => work(core.Remote()));
            done?.Invoke();
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
        busy = false;
        Render();
    }

    async void CreateInvitation()
    {
        if (busy)
        {
            return;
        }
        busy = true;
        Ui.Show(failure, "");
        Render();
        try
        {
            var created = await Task.Run(() => core.Remote().CreateInvitation(300));
            var png = new PngByteQRCode(new QRCodeGenerator().CreateQrCode(created.Encoded, QRCodeGenerator.ECCLevel.M)).GetGraphic(4);
            var image = new BitmapImage();
            using (var stream = new MemoryStream(png))
            {
                await image.SetSourceAsync(stream.AsRandomAccessStream());
            }
            invitation = created;
            code = image;
            copied = false;
        }
        catch (Exception problem)
        {
            Ui.Show(failure, Core.Describe(problem));
        }
        busy = false;
        Render();
    }

    void Connect()
    {
        var encoded = invite.Text.Trim();
        if (encoded.Length == 0)
        {
            return;
        }
        try
        {
            VeluneBindingsMethods.DecodeRemoteOffer(encoded);
        }
        catch (BindingException problem)
        {
            Ui.Show(failure, Core.Describe(problem));
            return;
        }
        invite.Text = "";
        Perform(endpoint => endpoint.BeginPairing(encoded));
    }

    void Forget(string peer, bool pending)
    {
        core.ForgetConnection(peer);
        Perform(endpoint =>
        {
            if (pending)
            {
                endpoint.CancelPairing(peer);
            }
            endpoint.ForgetDevice(peer);
        });
    }

    void Render()
    {
        if (invitation != null && Expired(invitation.ExpiresAtUnixSeconds))
        {
            invitation = null;
        }
        var state = hub.State;
        // The invite box keeps its focus and text across renders.
        var keep = invite.FocusState != FocusState.Unfocused;
        panel.Children.Clear();
        panel.Children.Add(Ui.Text("设备", "SubtitleTextBlockStyle"));
        Ui.Show(failure, failure.Message.Length > 0 ? failure.Message : hub.Failure);
        panel.Children.Add(failure);
        ThisPC(state);
        Others(state);
        if (keep)
        {
            invite.Focus(FocusState.Programmatic);
        }
    }

    void ThisPC(BindingRemoteServiceSnapshot? state)
    {
        panel.Children.Add(Ui.Heading("这台电脑"));
        var ingress = new ToggleSwitch { IsOn = state?.ControlIngress ?? false, IsEnabled = state != null && !busy, OnContent = "开", OffContent = "关" };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(ingress, "允许其他设备控制这台电脑");
        ingress.Toggled += (_, _) =>
        {
            if (ingress.IsOn != (hub.State?.ControlIngress ?? false))
            {
                var enabled = ingress.IsOn;
                Perform(endpoint => endpoint.SetControlIngress(enabled));
            }
        };
        panel.Children.Add(Ui.Card(new TextBlock { Text = "允许其他设备控制这台电脑" }, ingress));
        panel.Children.Add(Ui.Card(new TextBlock { Text = "设备名称" }, Ui.Muted(hub.DeviceName(Device.Local))));
        if (invitation != null)
        {
            panel.Children.Add(InvitationCard(invitation));
        }
        else
        {
            var add = new Button { Content = "添加控制设备…", IsEnabled = state != null && !busy };
            add.Click += (_, _) => CreateInvitation();
            panel.Children.Add(Ui.Card(Ui.TwoLines("添加控制设备", "在另一台设备上扫描二维码或粘贴邀请，两台设备显示的核对码一致时，在这台电脑上允许。", wrap: true), add));
        }
        foreach (var grant in state?.IncomingGrants ?? [])
        {
            var revoke = new Button { Content = "撤销", IsEnabled = !busy };
            var peer = grant.PeerId;
            revoke.Click += (_, _) => Perform(endpoint => endpoint.RevokeControl(peer));
            panel.Children.Add(Ui.Card(Ui.TwoLines(grant.Name, "可以控制这台电脑"), revoke));
        }
    }

    Border InvitationCard(BindingRemoteInvitation shown)
    {
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(new TextBlock { Text = "在另一台设备上扫描二维码，或复制邀请粘贴到那台设备。", TextWrapping = TextWrapping.Wrap });
        body.Children.Add(Ui.Muted("到期：" + DateTimeOffset.FromUnixTimeSeconds((long)shown.ExpiresAtUnixSeconds).ToLocalTime().ToString("HH:mm")));
        var copy = new Button { Content = copied ? "已复制" : "复制邀请" };
        copy.Click += (_, _) =>
        {
            var package = new DataPackage();
            package.SetText(shown.Encoded);
            Clipboard.SetContent(package);
            copied = true;
            Render();
        };
        var finish = new Button { Content = "完成" };
        finish.Click += (_, _) =>
        {
            invitation = null;
            Render();
        };
        body.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { copy, finish } });
        var row = new Grid { ColumnSpacing = 16 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        if (code != null)
        {
            // The code keeps its light quiet zone in either theme, as scanners need.
            row.Children.Add(new Border
            {
                Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.White),
                CornerRadius = new CornerRadius(4),
                Child = new Image { Source = code, Width = 180, Height = 180 },
            });
        }
        Grid.SetColumn(body, 1);
        row.Children.Add(body);
        return Ui.Card(row);
    }

    void Others(BindingRemoteServiceSnapshot? state)
    {
        panel.Children.Add(Ui.Heading("控制其他设备"));
        foreach (var contact in state?.Contacts ?? [])
        {
            var confirmed = contact.Confirmation is BindingContactConfirmation.Confirmed;
            Button action;
            if (confirmed)
            {
                action = new Button { Content = "打开" };
                action.Click += (_, _) => open(new Device(contact.PeerId));
            }
            else
            {
                action = new Button { Content = "取消", IsEnabled = !busy };
                action.Click += (_, _) => Forget(contact.PeerId, true);
            }
            var card = Ui.Card(Ui.TwoLines(contact.Name, ContactStatus(state!, contact.PeerId)), action);
            card.ContextFlyout = Ui.Menu(Ui.MenuItem("移除此设备", () => Forget(contact.PeerId, !confirmed)));
            panel.Children.Add(card);
        }
        connect.IsEnabled = Ready;
        panel.Children.Add(inviteCard);
    }

    // InviteCard holds the invite box and its button. It is built once and
    // only ever a child of panel, which Render clears, so it keeps the box's
    // text and focus without moving the box between parents.
    Border InviteCard()
    {
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.Children.Add(invite);
        Grid.SetColumn(connect, 1);
        row.Children.Add(connect);
        return Ui.Card(Ui.Field(row, "在要控制的设备上选择添加控制设备，把它显示的邀请粘贴到这里。连接后核对两台设备的核对码，并在那台设备上允许。"));
    }

    // ContactStatus is where pairing with a device stands, or its connection
    // once paired, in Mac's words.
    static string ContactStatus(BindingRemoteServiceSnapshot state, string peer)
    {
        foreach (var pairing in state.OutgoingPairings.Where(pairing => pairing.PeerId == peer && pairing.Phase != BindingPairingPhase.Granted))
        {
            return pairing.Phase switch
            {
                BindingPairingPhase.Rejected => "对方已拒绝",
                BindingPairingPhase.Expired => "邀请已过期",
                BindingPairingPhase.Disconnected => "配对时连接中断",
                _ => "等待对方允许 · 核对码 " + Hub.FormatCode(pairing.VerificationCode),
            };
        }
        foreach (var controller in state.Controllers.Where(controller => controller.PeerId == peer))
        {
            switch (controller.Phase)
            {
                case BindingControllerPhase.Connected:
                    return "已连接";
                case BindingControllerPhase.Connecting or BindingControllerPhase.Discovering:
                    return "正在连接";
                case BindingControllerPhase.Recovering:
                    return "正在重新连接";
                case BindingControllerPhase.Blocked:
                    return controller.ErrorCode is { } error ? $"无法连接（{error}）" : "无法连接";
            }
        }
        return "未连接";
    }
}
