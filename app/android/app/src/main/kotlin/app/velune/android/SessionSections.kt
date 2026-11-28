package app.velune.android

import app.velune.bindings.application.ActiveConversation
import app.velune.bindings.application.ListedConversation
import app.velune.bindings.conversation.RunState

/** One conversation in the merged list, with the device it runs on. */
data class SessionRow(
    val peer: String,
    val device: String,
    val listed: ListedConversation,
    val runtime: RuntimeLabel?,
    val active: ActiveConversation?,
    /** The project's display name, or null for a conversation without one. */
    val project: String?,
) {
    val key: String get() = peer + "/" + listed.conversation.runtimeId + "/" + listed.conversation.id
    val title: String get() = listed.conversation.title.displayText()
    val waiting: Boolean get() = (active?.pendingInteractions ?: 0u) > 0u
    val running: Boolean get() = active?.runState == RunState.RUNNING || active?.runState == RunState.STOPPING
    fun target() = ConversationTarget.Existing(listed.conversation.runtimeId, listed.conversation.id, title)
}

enum class SessionSectionKind { Waiting, Pinned, Recent }

data class SessionSection(val kind: SessionSectionKind, val rows: List<SessionRow>)

/**
 * Merges the lists of every shown device into one, as the Mac's sidebar does:
 * conversations that wait for an answer first, then pinned ones, then the
 * rest by when they last changed. Grouping stays a reading aid; each row
 * keeps its device and runtime. `query` matches the title, project and device.
 */
fun sessionSections(listings: List<Pair<DeviceSummary, DeviceListing>>, query: String): List<SessionSection> {
    val needle = query.trim()
    val rows = listings.flatMap { (device, listing) ->
        listing.conversations.map { listed ->
            SessionRow(
                peer = device.peerId,
                device = device.name,
                listed = listed,
                runtime = listing.runtimeLabels[listed.conversation.runtimeId],
                active = listing.active[listed.conversation.id],
                project = listed.project?.let { listing.projectNames[it] ?: lastPathComponent(it) },
            )
        }
    }.filter { row ->
        needle.isEmpty() || row.title.contains(needle, ignoreCase = true) ||
            row.project?.contains(needle, ignoreCase = true) == true || row.device.contains(needle, ignoreCase = true)
    }.sortedByDescending { it.listed.conversation.updatedAtUnixMs ?: Long.MIN_VALUE }
    val (waiting, rest) = rows.partition { it.waiting }
    val (pinned, recent) = rest.partition { it.listed.pinned }
    return listOf(
        SessionSection(SessionSectionKind.Waiting, waiting),
        SessionSection(SessionSectionKind.Pinned, pinned),
        SessionSection(SessionSectionKind.Recent, recent),
    ).filter { it.rows.isNotEmpty() }
}
