using Microsoft.UI.Dispatching;
using Velune.Bindings;

namespace Velune;

// The remote service's state, shared by every window. It changes on the UI
// thread only.
sealed class Hub(Core core, DispatcherQueue dispatcher)
{
    public Core Core { get; } = core;
    public BindingRemoteServiceSnapshot? State { get; private set; }
    public string Failure { get; private set; } = "";
    // Changed runs on the UI thread after each new state.
    public event Action? Changed;

    // Start opens the remote service and follows its state until the app quits.
    public void Start()
    {
        var thread = new Thread(Follow) { IsBackground = true, Name = "Velune remote state" };
        thread.Start();
    }

    void Follow()
    {
        VeluneRemoteStateObserver observer;
        try
        {
            var endpoint = Core.OpenEndpoint();
            Apply(endpoint.ReadState());
            observer = endpoint.ObserveState();
        }
        catch (BindingException failure)
        {
            dispatcher.TryEnqueue(() =>
            {
                Failure = "远程控制服务无法启动：" + Core.Describe(failure);
                Problems.Report("启动远程控制服务", failure);
                Changed?.Invoke();
            });
            return;
        }
        while (true)
        {
            try
            {
                if (observer.Next(5_000) is { } next)
                {
                    Apply(next);
                }
            }
            catch (BindingException.Closed)
            {
                return;
            }
            catch (BindingException)
            {
                Thread.Sleep(1_000);
            }
        }
    }

    void Apply(BindingRemoteServiceSnapshot state) => dispatcher.TryEnqueue(() =>
    {
        State = state;
        Changed?.Invoke();
    });

    // Devices are the paired devices this PC can open: those the issuing
    // device has confirmed.
    public IEnumerable<BindingDeviceContact> Devices() =>
        State?.Contacts.Where(contact => contact.Confirmation is BindingContactConfirmation.Confirmed) ?? [];

    public string DeviceName(Device device)
    {
        if (device.IsLocal)
        {
            return State?.Descriptor?.Name ?? Core.DeviceName();
        }
        return State?.Contacts.FirstOrDefault(contact => contact.PeerId == device.PeerId)?.Name ?? "设备";
    }

    // SetActivity tells the core which device the user looks at, so it keeps
    // that connection ready.
    public void SetActivity(Device previous, Device current, bool foreground)
    {
        VeluneRemoteEndpoint endpoint;
        try
        {
            endpoint = Core.Remote();
        }
        catch (BindingException)
        {
            return;
        }
        _ = Task.Run(() =>
        {
            try
            {
                if (!previous.IsLocal && previous != current)
                {
                    endpoint.SetControllerActivity(previous.PeerId, false, false);
                }
                if (!current.IsLocal)
                {
                    endpoint.SetControllerActivity(current.PeerId, true, foreground);
                }
            }
            catch (BindingException)
            {
                // The next state shows where the connection stands.
            }
        });
    }

    // FormatCode groups the eight digits as four and four, as the other
    // device shows them.
    public static string FormatCode(string? code) =>
        code is { Length: 8 } ? code[..4] + " " + code[4..] : code ?? "";
}
