package app.velune.android

import app.velune.bindings.BindingListedConversation

/** Conversations under one heading: pinned ones, one project, or those without a project. */
data class ConversationSection(val key: String, val title: String, val path: String?, val conversations: List<BindingListedConversation>)

/**
 * Groups the device's list by project, as HAPI and Lody do on phones. Pinned
 * conversations lead; sections keep the order in which the core's list first
 * reaches them, so the most recent project comes first. `query` keeps the
 * conversations whose title or project contains it.
 */
fun conversationSections(conversations: List<BindingListedConversation>, projectNames: Map<String, String>, query: String): List<ConversationSection> {
    val needle = query.trim()
    fun projectTitle(path: String) = projectNames[path] ?: lastPathComponent(path)
    val shown = if (needle.isEmpty()) conversations else conversations.filter { listed ->
        listed.conversation.title.contains(needle, ignoreCase = true) || listed.project?.let { projectTitle(it).contains(needle, ignoreCase = true) } == true
    }
    val (pinned, rest) = shown.partition { it.pinned }
    val pinnedSection = if (pinned.isEmpty()) emptyList() else listOf(ConversationSection("pinned", "置顶", null, pinned))
    return pinnedSection + rest.groupBy { it.project }.map { (path, members) ->
        ConversationSection("project/" + (path ?: ""), path?.let(::projectTitle) ?: "未指定项目", path, members)
    }
}
