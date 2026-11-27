package app.velune.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.conversation.Collaborator
import app.velune.bindings.conversation.ConversationSummary

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun CollaborationSheet(
    state: ConversationState,
    runtimeId: String,
    runtimeLabels: Map<String, RuntimeLabel>,
    sessions: List<ConversationSummary>,
    runningIds: Set<String>,
    waitingIds: Set<String>,
    openFailure: String?,
    openingConversationId: String?,
    onDismiss: () -> Unit,
    onStop: () -> Unit,
    onOpen: (ConversationTarget.Existing) -> Unit,
    onOpenUnlisted: (String) -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    val sessionsById = remember(sessions) { sessions.associateBy { it.id } }
    val parentId = state.parentConversationId
    val linkedIds = mutableSetOf<String>()
    parentId?.let(linkedIds::add)
    val children = state.subAgents.filter { linkedIds.add(it.conversationId) }
        .sortedByDescending { it.conversationId in waitingIds || it.conversationId in runningIds }
    val collaborators = state.collaborators.filter { linkedIds.add(it.conversationId) }
    val showChildren = children.isNotEmpty() || state.canStopSubAgents

    fun open(id: String, title: String, native: Boolean) {
        val listed = sessionsById[id]
        // Native IDs already name their instance. An unlisted A2A link needs a core lookup.
        val nativeRuntime = id.substringBefore(':', missingDelimiterValue = "")
            .takeIf { native && it.isNotEmpty() && (it == runtimeId || it in runtimeLabels) }
        val sourceRuntime = listed?.runtimeId ?: nativeRuntime
        if (sourceRuntime == null) onOpenUnlisted(id)
        else onOpen(ConversationTarget.Existing(sourceRuntime, id, listed?.title?.displayText() ?: title))
    }

    ModalBottomSheet(
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        onDismissRequest = onDismiss,
    ) {
        Text(
            "Agent 协作",
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp).semantics { heading() },
        )
        if (openFailure != null) {
            Text(
                openFailure,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp * spacing)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
        LazyColumn(contentPadding = PaddingValues(bottom = 16.dp * spacing)) {
            if (parentId != null) {
                item(key = "parent-heading") { CollaborationHeading("父会话") }
                item(key = "link/$parentId") {
                    val title = sessionsById[parentId]?.title?.displayText() ?: "查看父会话"
                    LinkedConversationRow(parentId, title, openingConversationId) { open(parentId, title, native = true) }
                }
            }
            if (showChildren) {
                item(key = "children-heading") { CollaborationHeading("子 Agent") }
                items(children, key = { "link/${it.conversationId}" }) { agent ->
                    val id = agent.conversationId
                    val title = agent.title.ifBlank { sessionsById[id]?.title?.displayText() ?: "子 Agent" }
                    val (status, tone) = when {
                        id in waitingIds -> "等待处理" to Tone.Attention
                        id in runningIds -> "运行中" to Tone.Active
                        agent.failed -> "失败" to Tone.Negative
                        else -> "已完成" to Tone.Neutral
                    }
                    LinkedConversationRow(id, title, openingConversationId, status, tone) { open(id, title, native = true) }
                }
                if (state.canStopSubAgents) {
                    item(key = "stop-children") {
                        TextButton(
                            onClick = onStop,
                            enabled = !state.busy,
                            modifier = Modifier.padding(horizontal = 16.dp),
                        ) { Text("停止子 Agent") }
                    }
                }
            }
            if (collaborators.isNotEmpty()) {
                item(key = "collaborators-heading") { CollaborationHeading("协作会话") }
                items(collaborators, key = { "link/${it.conversationId}" }) { agent ->
                    val id = agent.conversationId
                    val listed = sessionsById[id]
                    val title = agent.title.ifBlank { listed?.title?.displayText() ?: "未命名会话" }
                    val name = agent.runtimeName?.takeIf { it.isNotBlank() }
                        ?: listed?.runtimeId?.let(runtimeLabels::get)?.name
                    val detail = listOfNotNull(name, agent.contactDirection()).joinToString(" · ").ifEmpty { null }
                    LinkedConversationRow(id, title, openingConversationId, detail) { open(id, title, native = false) }
                }
            }
            if (parentId == null && !showChildren && collaborators.isEmpty()) {
                item(key = "empty") {
                    Text(
                        "没有关联 Agent",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 24.dp, vertical = 20.dp * spacing),
                    )
                }
            }
        }
    }
}

@Composable
private fun CollaborationHeading(title: String) {
    val spacing = LocalAppearance.current.spacingScale
    Text(
        title,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        style = MaterialTheme.typography.labelLarge,
        modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 12.dp * spacing).semantics { heading() },
    )
}

@Composable
private fun LinkedConversationRow(
    id: String,
    title: String,
    openingId: String?,
    detail: String? = null,
    tone: Tone? = null,
    open: () -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    ListItem(
        headlineContent = { Text(title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = detail?.let { message ->
            {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(6.dp * spacing),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (tone != null) StatusDot(tone, size = 6.dp)
                    Text(message, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
        },
        trailingContent = {
            if (id == openingId) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            } else {
                Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null)
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.fillMaxWidth()
            .clickable(enabled = openingId == null, onClickLabel = "打开会话", onClick = open)
            .padding(horizontal = 8.dp),
    )
}

private fun Collaborator.contactDirection(): String? = when {
    contacted && contactedBy -> "互相联系"
    contacted -> "本会话联系了它"
    contactedBy -> "它联系了本会话"
    else -> null
}
