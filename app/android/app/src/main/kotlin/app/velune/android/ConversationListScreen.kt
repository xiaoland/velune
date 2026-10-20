package app.velune.android

import android.text.format.DateUtils
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingContactConfirmation
import app.velune.bindings.BindingListedConversation
import app.velune.bindings.BindingRunState
import app.velune.bindings.BindingRuntimeInstance

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationListScreen(
    hub: DeviceHub,
    workspace: DeviceWorkspace,
    back: () -> Unit,
    switchDevice: (String) -> Unit,
    open: (ConversationTarget) -> Unit,
) {
    val state by hub.state.collectAsState()
    val listing by workspace.listing.collectAsState()
    var switching by remember { mutableStateOf(false) }
    var starting by remember { mutableStateOf(false) }
    WhileVisible(workspace, resume = workspace::resume, pause = workspace::pause)
    val contact = state?.contacts?.firstOrNull { it.peerId == workspace.peer }
    val runtimeNames = listing.runtimes.associate { it.id to it.name }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "设备") } },
                title = {
                    Box {
                        Row(Modifier.clickable { switching = true }, verticalAlignment = Alignment.CenterVertically) {
                            Column {
                                Text(contact?.name ?: "设备", maxLines = 1, overflow = TextOverflow.Ellipsis)
                                if (state != null && contact != null) {
                                    Text(deviceStatus(state!!, contact, listing.path), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                            Icon(Icons.Filled.ArrowDropDown, contentDescription = "切换设备")
                        }
                        DropdownMenu(expanded = switching, onDismissRequest = { switching = false }) {
                            state?.contacts?.filter { it.confirmation is BindingContactConfirmation.Confirmed }?.forEach { other ->
                                DropdownMenuItem(
                                    text = { Text(other.name) },
                                    leadingIcon = { RadioButton(selected = other.peerId == workspace.peer, onClick = null) },
                                    onClick = { switching = false; if (other.peerId != workspace.peer) switchDevice(other.peerId) },
                                )
                            }
                        }
                    }
                },
            )
        },
        floatingActionButton = {
            if (listing.runtimes.isNotEmpty()) {
                ExtendedFloatingActionButton(onClick = { starting = true }, icon = { Icon(Icons.Filled.Add, contentDescription = null) }, text = { Text("新会话") })
            }
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                !listing.loaded -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                listing.conversations.isEmpty() -> Message(listing.failure ?: "这台设备上还没有会话。")
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    listing.failure?.let { failure -> item { Text(failure, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp)) } }
                    items(listing.conversations, key = { it.conversation.runtimeId + "/" + it.conversation.id }) { listed ->
                        ConversationRow(listed, runtimeNames[listed.conversation.runtimeId], listing.active[listed.conversation.id]?.let { it.runState to it.pendingInteractions }) {
                            open(ConversationTarget.Existing(listed.conversation.runtimeId, listed.conversation.id, listed.conversation.title))
                        }
                    }
                }
            }
        }
    }
    if (starting) {
        NewConversationDialog(listing.runtimes, listing.projects, dismiss = { starting = false }) { runtime, cwd ->
            starting = false
            open(ConversationTarget.New(runtime, cwd))
        }
    }
}

@Composable
private fun ConversationRow(listed: BindingListedConversation, runtime: String?, active: Pair<BindingRunState, UInt>?, open: () -> Unit) {
    val conversation = listed.conversation
    val detail = listOfNotNull(
        runtime,
        conversation.updatedAtUnixMs?.let { DateUtils.getRelativeTimeSpanString(it, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString() },
        listed.project?.substringAfterLast('/')?.substringAfterLast('\\'),
    ).joinToString(" · ")
    ListItem(
        headlineContent = { Text(conversation.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(detail, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = {
            when {
                active != null && active.second > 0u -> Text("等待回应", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.labelMedium)
                active?.first == BindingRunState.RUNNING -> CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                listed.failed -> Text("失败", color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.labelMedium)
                listed.unread -> Text("●", color = MaterialTheme.colorScheme.primary)
            }
        },
        modifier = Modifier.clickable(onClick = open),
    )
}

@Composable
private fun NewConversationDialog(runtimes: List<BindingRuntimeInstance>, projects: List<String>, dismiss: () -> Unit, start: (String, String?) -> Unit) {
    var runtime by remember { mutableStateOf(runtimes.first().id) }
    var project by remember { mutableStateOf(projects.firstOrNull()) }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text("新会话") },
        text = {
            LazyColumn {
                item { Text("运行时", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(vertical = 8.dp)) }
                items(runtimes, key = { "r/" + it.id }) { instance ->
                    Choice(instance.name, instance.id == runtime) { runtime = instance.id }
                }
                item { Text("项目", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 16.dp, bottom = 8.dp)) }
                items(projects, key = { "p/$it" }) { path ->
                    Choice(path, path == project) { project = path }
                }
                item { Choice("不指定（运行时默认目录）", project == null) { project = null } }
            }
        },
        confirmButton = { TextButton(onClick = { start(runtime, project) }) { Text("开始") } },
        dismissButton = { TextButton(onClick = dismiss) { Text("取消") } },
    )
}

@Composable
private fun Choice(label: String, selected: Boolean, select: () -> Unit) {
    Row(Modifier.fillMaxWidth().selectable(selected, onClick = select).padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        RadioButton(selected = selected, onClick = null)
        Text(label, modifier = Modifier.padding(start = 12.dp), maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}
