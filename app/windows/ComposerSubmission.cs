namespace Velune;

// One submission intent for the current editor, separate from stop controls.
readonly record struct ComposerSubmission(ComposerSubmission.Kind Action, bool Enabled)
{
    public enum Phase { Idle, Running, Stopping, Sending, Resuming, Blocked }
    public enum Kind { Send, Queue, Resume, Stopping, Sending, Resuming, None }
    public string Title => Action switch
    {
        Kind.Send => "发送", Kind.Queue => "排队", Kind.Resume => "继续",
        Kind.Stopping => "正在停止…", Kind.Sending => "正在发送…", Kind.Resuming => "正在恢复…", _ => ""
    };
    public bool CanSubmitDraft => Enabled && Action is Kind.Send or Kind.Queue;
    public bool CanResume => Enabled && Action == Kind.Resume;

    public static ComposerSubmission Resolve(Phase phase, bool hasInput, bool composing, bool canSend, bool canQueue, bool canResume) => phase switch
    {
        Phase.Stopping => new(Kind.Stopping, false),
        Phase.Sending => new(Kind.Sending, false),
        Phase.Resuming => new(Kind.Resuming, false),
        Phase.Blocked => new(Kind.Send, false),
        Phase.Running => new(hasInput ? Kind.Queue : Kind.None, hasInput && canQueue && !composing),
        _ => new(hasInput ? Kind.Send : canResume ? Kind.Resume : Kind.Send, !composing && canSend && (hasInput || canResume))
    };
}
