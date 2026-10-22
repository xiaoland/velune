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
    }

    readonly Dictionary<Device, Feed> feeds = [];

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

    public void Fail(Device device, Exception failure)
    {
        Of(device).Failure = Core.Describe(failure);
        Notify(device);
    }

    async void Poll(Device device, Feed feed)
    {
        var current = ++feed.Round;
        ulong cursor = 0;
        while (feed.Round == current)
        {
            var delay = 5_000;
            var changes = feed.Written;
            try
            {
                var (list, activity) = await Task.Run(() =>
                {
                    var application = core.Application(device);
                    return (application.List(), application.ConversationActivity(cursor));
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
                cursor = activity.Cursor;
                feed.Listing = Listing.From(list, activity.Active);
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
                feed.Listing = feed.Listing.Failed(Core.Describe(failure));
            }
            Notify(device);
            await Task.Delay(delay);
        }
    }

    // Change runs one change of the device's list and shows the list it returns.
    public async void Change(Device device, Func<VeluneApplication, BindingConfigurationSnapshot> work)
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
            feed.Listing.Failure = Core.Describe(failure);
        }
        feed.Writing--;
        feed.Written++;
        Notify(device);
    }

    // Write runs a change whose result the next read shows, such as the
    // list's preferences or a conversation's title.
    public async void Write(Device device, Action<VeluneApplication> work)
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
            feed.Listing.Failure = Core.Describe(failure);
            Notify(device);
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
            feed.Listing.Failure = Core.Describe(failure);
        }
        feed.MarkingRead.Remove(id);
        Notify(device);
    }

    public bool MarkingRead(Device device, string id) => feeds.TryGetValue(device, out var feed) && feed.MarkingRead.Contains(id);
}
