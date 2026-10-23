package app.velune.android

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.SearchOff
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingListedConversation
import app.velune.bindings.BindingRunState
import kotlinx.coroutines.launch

class ConversationListActions(
    val back: () -> Unit,
    val switchDevice: (String) -> Unit,
    val open: (ConversationTarget) -> Unit,
    val refresh: () -> Unit,
)

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
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    WhileVisible(workspace, resume = workspace::resume, pause = workspace::pause)
    val devices = state?.let { snapshot ->
        snapshot.contacts.map { deviceSummary(snapshot, it, if (it.peerId == workspace.peer) listing.path else null) }
    }.orEmpty()
    val device = devices.firstOrNull { it.peerId == workspace.peer } ?: DeviceSummary(workspace.peer, "设备", DeviceStatus("未连接", Tone.Neutral), true)
    ConversationListContent(
        device, devices.filter { it.confirmed }, listing, refreshing,
        ConversationListActions(back, switchDevice, open, refresh = {
            refreshing = true
            scope.launch { workspace.refresh(); refreshing = false }
        }),
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ConversationListContent(device: DeviceSummary, devices: List<DeviceSummary>, listing: DeviceListing, refreshing: Boolean, actions: ConversationListActions) {
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var starting by remember { mutableStateOf(false) }
    val collapsed = remember { mutableStateListOf<String>() }
    val list = rememberLazyListState()
    val expandedFab by remember { derivedStateOf { list.firstVisibleItemIndex == 0 } }
    val sections = remember(listing.conversations, listing.projectNames, query) { conversationSections(listing.conversations, listing.projectNames, query) }
    Scaffold(
        topBar = {
            if (searching) SearchBar(query, { query = it }, close = { searching = false; query = "" })
            else TopAppBar(
                navigationIcon = { IconButton(onClick = actions.back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "设备") } },
                title = { DeviceSwitcher(device, devices, actions.switchDevice) },
                actions = { IconButton(onClick = { searching = true }) { Icon(Icons.Filled.Search, contentDescription = "搜索会话") } },
            )
        },
        floatingActionButton = {
            if (listing.runtimes.isNotEmpty()) ExtendedFloatingActionButton(
                onClick = { starting = true }, expanded = expandedFab,
                icon = { Icon(Icons.Filled.Add, contentDescription = null) }, text = { Text("新会话") },
            )
        },
    ) { padding ->
        PullToRefreshBox(isRefreshing = refreshing, onRefresh = actions.refresh, modifier = Modifier.fillMaxSize().padding(padding)) {
            when {
                !listing.loaded -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                listing.conversations.isEmpty() && listing.failure != null -> EmptyState(Icons.Outlined.CloudOff, "无法读取会话", listing.failure)
                listing.conversations.isEmpty() -> EmptyState(Icons.Outlined.ChatBubbleOutline, "还没有会话", "在这台设备上开始第一个会话。") {
                    if (listing.runtimes.isNotEmpty()) Button(onClick = { starting = true }) { Text("新会话") }
                }
                sections.isEmpty() -> EmptyState(Icons.Outlined.SearchOff, "没有匹配“$query”的会话")
                else -> LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                    listing.failure?.let { failure -> item(key = "failure") { Banner(failure) } }
                    sections.forEach { section ->
                        val folded = section.key in collapsed && query.isEmpty()
                        stickyHeader(key = "h/" + section.key) {
                            SectionHeader(section, folded) { if (folded) collapsed.remove(section.key) else collapsed.add(section.key) }
                        }
                        if (!folded) items(section.conversations, key = { section.key + "/" + it.conversation.runtimeId + "/" + it.conversation.id }) { listed ->
                            ConversationRow(
                                listed, listing.runtimeLabels[listed.conversation.runtimeId], listing.active[listed.conversation.id],
                                showProject = section.path == null, projectNames = listing.projectNames,
                            ) { actions.open(ConversationTarget.Existing(listed.conversation.runtimeId, listed.conversation.id, listed.conversation.title)) }
                        }
                    }
                }
            }
        }
    }
    if (starting) NewConversationSheet(listing, dismiss = { starting = false }) { runtime, cwd ->
        starting = false
        actions.open(ConversationTarget.New(runtime, cwd))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchBar(query: String, change: (String) -> Unit, close: () -> Unit) {
    TopAppBar(
        navigationIcon = { IconButton(onClick = close) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "结束搜索") } },
        title = {
            TextField(
                value = query, onValueChange = change, placeholder = { Text("搜索标题或项目") }, singleLine = true, modifier = Modifier.fillMaxWidth(),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent,
                ),
            )
        },
        actions = { if (query.isNotEmpty()) IconButton(onClick = { change("") }) { Icon(Icons.Filled.Close, contentDescription = "清除") } },
    )
}

/** The device name in the title opens the other devices: one interface, switched to another device. */
@Composable
private fun DeviceSwitcher(device: DeviceSummary, devices: List<DeviceSummary>, switchDevice: (String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Row(Modifier.clickable(enabled = devices.size > 1) { open = true }, verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f, fill = false)) {
                Text(device.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    StatusDot(device.status.tone, size = 6.dp)
                    Text(device.status.text, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, modifier = Modifier.padding(start = 5.dp))
                }
            }
            if (devices.size > 1) Icon(Icons.Filled.ArrowDropDown, contentDescription = "切换设备")
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            devices.forEach { other ->
                DropdownMenuItem(
                    text = { Text(other.name, fontWeight = if (other.peerId == device.peerId) FontWeight.SemiBold else null) },
                    leadingIcon = { StatusDot(other.status.tone) },
                    onClick = { open = false; if (other.peerId != device.peerId) switchDevice(other.peerId) },
                )
            }
        }
    }
}

@Composable
private fun SectionHeader(section: ConversationSection, folded: Boolean, toggle: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).clickable(onClick = toggle).padding(start = 16.dp, end = 12.dp, top = 14.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(if (section.path == null && section.key == "pinned") Icons.Outlined.PushPin else Icons.Outlined.Folder, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(section.title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 8.dp).weight(1f))
        Text("${section.conversations.size}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
        Icon(Icons.Filled.KeyboardArrowDown, contentDescription = if (folded) "展开" else "收起", modifier = Modifier.size(18.dp).rotate(if (folded) -90f else 0f), tint = MaterialTheme.colorScheme.outline)
    }
}

/**
 * Two lines like HAPI's rows: runtime, title and age; then the runtime's
 * name and whatever needs attention. Unread titles are bold.
 */
@Composable
private fun ConversationRow(
    listed: BindingListedConversation,
    runtime: RuntimeLabel?,
    active: app.velune.bindings.BindingActiveConversation?,
    showProject: Boolean,
    projectNames: Map<String, String>,
    open: () -> Unit,
) {
    val conversation = listed.conversation
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth().clickable(onClick = open).padding(horizontal = 16.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RuntimeBadge(runtime)
            Text(
                conversation.title, style = MaterialTheme.typography.bodyLarge, fontWeight = if (listed.unread) FontWeight.SemiBold else null,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 10.dp).weight(1f),
            )
            conversation.updatedAtUnixMs?.let {
                Text(relativeTime(it), style = MaterialTheme.typography.labelSmall, color = colors.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp))
            }
        }
        Row(Modifier.padding(start = 28.dp), verticalAlignment = Alignment.CenterVertically) {
            val meta = listOfNotNull(runtime?.name, listed.project?.takeIf { showProject }?.let { projectNames[it] ?: lastPathComponent(it) }).joinToString(" · ")
            Text(meta, style = MaterialTheme.typography.bodySmall, color = colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            when {
                active != null && active.pendingInteractions > 0u -> Pill("等待回应", colors.tertiaryContainer, colors.onTertiaryContainer)
                active?.runState == BindingRunState.RUNNING -> Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp, color = Tone.Positive.color())
                    Text("运行中", style = MaterialTheme.typography.labelSmall, color = Tone.Positive.color(), modifier = Modifier.padding(start = 4.dp))
                }
                listed.failed -> Pill("失败", colors.errorContainer, colors.onErrorContainer)
                listed.unread -> StatusDot(Tone.Active)
            }
        }
    }
}

@Composable
private fun Banner(text: String) {
    Surface(color = MaterialTheme.colorScheme.errorContainer, contentColor = MaterialTheme.colorScheme.onErrorContainer, modifier = Modifier.fillMaxWidth()) {
        Text(text, style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
    }
}

/** Where a new conversation runs: a runtime, and a project folder or the runtime's own default. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NewConversationSheet(listing: DeviceListing, dismiss: () -> Unit, start: (String, String?) -> Unit) {
    var runtime by remember { mutableStateOf(listing.runtimes.first().id) }
    var project by remember { mutableStateOf(listing.projects.firstOrNull()) }
    ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = dismiss) {
        Column(Modifier.navigationBarsPadding().padding(bottom = 16.dp)) {
            Text("新会话", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp))
            Text("运行时", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 8.dp))
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listing.runtimes.forEach { instance ->
                    FilterChip(
                        selected = instance.id == runtime, onClick = { runtime = instance.id },
                        label = { Text(instance.name) }, leadingIcon = { RuntimeBadge(listing.runtimeLabels[instance.id], Modifier.size(16.dp)) },
                    )
                }
            }
            Text("项目", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 4.dp))
            LazyColumn(Modifier.weight(1f, fill = false)) {
                items(listing.projects, key = { "p/$it" }) { path ->
                    ProjectChoice(listing.projectNames[path] ?: lastPathComponent(path), path, path == project) { project = path }
                }
                item(key = "none") { ProjectChoice("不指定", "使用运行时默认目录", project == null) { project = null } }
            }
            Spacer(Modifier.size(8.dp))
            Button(onClick = { start(runtime, project) }, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp)) { Text("开始") }
        }
    }
}

@Composable
private fun ProjectChoice(title: String, detail: String, selected: Boolean, select: () -> Unit) {
    Row(Modifier.fillMaxWidth().selectable(selected, onClick = select).padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        RadioButton(selected = selected, onClick = null, modifier = Modifier.padding(horizontal = 8.dp))
        Column(Modifier.padding(start = 8.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}
