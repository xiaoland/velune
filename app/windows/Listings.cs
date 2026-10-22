using Velune.Bindings;

namespace Velune;

// The conversation list of each device a window shows, read once for all of
// them: quickly while a conversation runs, slowly otherwise. It changes on
// the UI thread only.
sealed class Listings(Core core)
{
    sealed class Feed
    {
        public Listing Listing = Listing.Loading;
        public readonly List<Action> Followers = [];
        // A new round replaces the reading loop; the old one ends.
        public int Round;
        // Changes under way and those finished: a list read while one was
        // under way may show the state before it, so it is not shown.
        public int Writing;
        public int Written;
        // Conversations whose read mark is on its way.
        public readonly HashSet<string> MarkingRead = [];
        // Where the device's ended turns were read to, across rounds, and
        // whether the list was read yet.
        public ulong Cursor;
        public bool Read;
    }

    readonly Dictionary<Device, Feed> feeds = [];

    // Polled tells each new list of a device with the turns that ended since
    // the last, and whether it is the device's first list.
    public event Action<Device, Listing, BindingFinishedTurn[], bool>? Polled;

    public Listing Of(Device device) => feeds.TryGetValue(device, out var feed) ? feed.Listing : Listing.Loading;

    // Follow calls changed after each new list of the device until Leave.
    public void Follow(Device device, Action changed)
    {
        if (!feeds.TryGetValue(device, out var feed))
        {
            feed = new Feed();
            feeds[device] = feed;
        }
        feed.Followers.Add(changed);
        if (feed.Followers.Count == 1)
        {
            Poll(device, feed);
        }
    }

    public void Leave(Device device, Action changed)
    {
        if (!feeds.TryGetValue(device, out var feed))
        {
            return;
        }
        feed.Followers.Remove(changed);
        if (feed.Followers.Count == 0)
        {
            feed.Round++;
            feeds.Remove(device);
        }
    }

    // Reread reads the device's list now.
    public void Reread(Device device)
    {
        if (feeds.TryGetValue(device, out var feed))
        {
            Poll(device, feed);
        }
    }

    // Notify shows a change made to the list in place to every follower.
    public void Notify(Device device)
    {
        if (feeds.TryGetValue(device, out var feed))
        {
            foreach (var changed in feed.Followers.ToList())
            {
                changed();
            }
        }
    }


    async void Poll(Device device, Feed feed)
    {
        var current = ++feed.Round;
        while (feed.Round == current)
        {
            (BindingFinishedTurn[] Turns, bool First)? ended = null;
            var delay = 5_000;
            var changes = feed.Written;
            try
            {
                var (list, activity) = await Task.Run(() =>
                {
                    var application = core.Application(device);
                    return (application.List(), application.ConversationActivity(feed.Cursor));
                });
                if (feed.Round != current)
                {
                    return;
                }
                if (feed.Writing > 0 || feed.Written != changes)
                {
                    // A change of the list was under way; the next read shows it.
                    await Task.Delay(500);
                    continue;
                }
                feed.Cursor = activity.Cursor;
                feed.Listing = Listing.From(list, activity.Active);
                Problems.Recovered(Problems.Key("list", device));
                var first = !feed.Read;
                feed.Read = true;
                ended = (activity.Finished, first);
                if (activity.Active.Length > 0)
                {
                    delay = 2_000;
                }
            }
            catch (Exception failure)
            {
                if (feed.Round != current)
                {
                    return;
                }
                feed.Listing = feed.Listing.Failed();
                Problems.Failing(Problems.Key("list", device), "读取会话列表", failure, device);
            }
            Notify(device);
            if (ended is var (turns, firstList))
            {
                Polled?.Invoke(device, feed.Listing, turns, firstList);
            }
            await Task.Delay(delay);
        }
    }

    // Change runs one change of the device's list and shows the list it
    // returns; a failure goes to 问题 under source.
    public async void Change(Device device, string source, Func<VeluneApplication, BindingConfigurationSnapshot> work)
    {
        if (!feeds.TryGetValue(device, out var feed))
        {
            return;
        }
        feed.Writing++;
        try
        {
            var list = await Task.Run(() => work(core.Application(device)));
            feed.Listing = Listing.From(list, [.. feed.Listing.Active.Values]);
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure, device);
        }
        feed.Writing--;
        feed.Written++;
        Notify(device);
    }

    // Write runs a change whose result the next read shows, such as the
    // list's preferences or a conversation's title.
    public async void Write(Device device, string source, Action<VeluneApplication> work)
    {
        if (!feeds.TryGetValue(device, out var feed))
        {
            return;
        }
        feed.Writing++;
        try
        {
            await Task.Run(() => work(core.Application(device)));
        }
        catch (Exception failure)
        {
            Problems.Report(source, failure, device);
        }
        feed.Writing--;
        feed.Written++;
    }

    // MarkRead marks a conversation read once a window shows it in front.
    public async void MarkRead(Device device, BindingListedConversation listed)
    {
        var id = listed.Conversation.Id;
        if (!feeds.TryGetValue(device, out var feed) || !feed.MarkingRead.Add(id))
        {
            return;
        }
        var reference = new BindingConversationReference(listed.Conversation.RuntimeId, id);
        try
        {
            await Task.Run(() => core.Application(device).OrganizeConversations([reference], new BindingConversationOrganizationChange.MarkRead()));
            var conversations = feed.Listing.Conversations;
            if (conversations.FindIndex(shown => shown.Conversation.Id == id) is var at and >= 0)
            {
                conversations[at] = conversations[at] with { Unread = false };
            }
        }
        catch (Exception failure)
        {
            Problems.Report("标为已读", failure, device);
        }
        feed.MarkingRead.Remove(id);
        Notify(device);
    }

    public bool MarkingRead(Device device, string id) => feeds.TryGetValue(device, out var feed) && feed.MarkingRead.Contains(id);
}
