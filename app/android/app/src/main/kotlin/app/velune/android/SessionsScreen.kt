package app.velune.android

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
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.SearchOff
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.remote.ContactConfirmation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch

class SessionsActions(
    val open: (peer: String, ConversationTarget) -> Unit,
    val refresh: () -> Unit,
    /** Narrows the list to one device; null shows every device. */
    val filter: (String?) -> Unit,
    val devices: () -> Unit,
    val add: () -> Unit,
    val telemetry: () -> Unit,
    val about: () -> Unit,
)

/**
 * The app's home: the conversations of every paired device in one list, as
 * the Mac's sidebar merges this Mac and its controlled devices. Devices are
 * where a conversation runs, not a level the user browses through.
 */
@Composable
fun SessionsScreen(
    hub: DeviceHub,
    workspace: (String) -> DeviceWorkspace,
    filter: String?,
    actions: SessionsActions,
) {
    val state by hub.state.collectAsState()
    val failure by hub.failure.collectAsState()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    val snapshot = state
    val contacts = snapshot?.contacts?.filter { it.controlAuthorized && it.confirmation is ContactConfirmation.Confirmed }.orEmpty()
    // A removed device no longer narrows the list.
    val effective = filter?.takeIf { peer -> contacts.any { it.peerId == peer } }
    val shown = contacts.map { it.peerId }.filter { effective == null || it == effective }.toSet()
    LaunchedEffect(shown) { hub.view(shown) }
    val listings = contacts.map { contact ->
        key(contact.peerId) {
            val owner = workspace(contact.peerId)
            val listing by owner.listing.collectAsState()
            if (contact.peerId in shown) WhileVisible(owner, resume = owner::resume, pause = owner::pause)
            deviceSummary(checkNotNull(snapshot), contact, listing.path) to listing
        }
    }
    SessionsContent(
        devices = if (snapshot == null) null else listings.map { it.first },
        listings = listings,
        filter = effective,
        failure = failure,
        refreshing = refreshing,
        actions = SessionsActions(
            open = actions.open, filter = actions.filter, devices = actions.devices, add = actions.add,
            telemetry = actions.telemetry, about = actions.about,
            refresh = {
                refreshing = true
                scope.launch {
                    coroutineScope { shown.forEach { peer -> launch { workspace(peer).refresh() } } }
                    refreshing = false
                }
            },
        ),
    )
}

/** `devices` is null while the remote service starts. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionsContent(
    devices: List<DeviceSummary>?,
    listings: List<Pair<DeviceSummary, DeviceListing>>,
    filter: String?,
    failure: String?,
    refreshing: Boolean,
    actions: SessionsActions,
) {
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var starting by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    val expandedFab by remember { derivedStateOf { list.firstVisibleItemIndex == 0 } }
    val shown = listings.filter { filter == null || it.first.peerId == filter }
    val sections = remember(shown, query) { sessionSections(shown, query) }
    val unreachable = shown.filter { it.second.failure != null }
    val startable = listings.filter { it.second.runtimes.isNotEmpty() }
    val multipleDevices = (devices?.size ?: 0) > 1
    Scaffold(
        topBar = {
            Column {
                if (searching) SearchBar(query, { query = it }, close = { searching = false; query = "" })
                else TopAppBar(
                    title = { Text("会话") },
                    actions = {
                        if (!devices.isNullOrEmpty()) IconButton(onClick = { searching = true }) { Icon(Icons.Filled.Search, contentDescription = "搜索会话") }
                        OverflowMenu(devicesReady = devices != null, actions)
                    },
                )
                if (multipleDevices && !searching) DeviceFilter(checkNotNull(devices), filter, actions.filter)
            }
        },
        floatingActionButton = {
            if (startable.isNotEmpty() && !searching) ExtendedFloatingActionButton(
                onClick = { starting = true }, expanded = expandedFab,
                icon = { Icon(Icons.Filled.Add, contentDescription = null) }, text = { Text("新会话") },
            )
        },
    ) { padding ->
        PullToRefreshBox(isRefreshing = refreshing, onRefresh = actions.refresh, modifier = Modifier.fillMaxSize().padding(padding)) {
            when {
                failure != null -> EmptyState(Icons.Outlined.ErrorOutline, "远程控制没有启动", failure)
                devices == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                devices.isEmpty() -> EmptyState(Icons.Outlined.Devices, "连接你的电脑", "在电脑的“设置 › 设备”中添加控制设备，然后扫码或打开邀请链接。") {
                    Button(onClick = actions.add) { Text("添加设备") }
                }
                sections.isEmpty() && query.isNotBlank() -> EmptyState(Icons.Outlined.SearchOff, "没有匹配“${query.trim()}”的会话")
                sections.isEmpty() && shown.any { !it.second.loaded } -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                sections.isEmpty() && unreachable.size == shown.size -> EmptyState(Icons.Outlined.CloudOff, "无法读取会话", unreachable.joinToString("\n") { "${it.first.name}：${it.second.failure}" })
                sections.isEmpty() && unreachable.isEmpty() -> EmptyState(Icons.Outlined.ChatBubbleOutline, "还没有会话") {
                    if (startable.isNotEmpty()) Button(onClick = { starting = true }) { Text("新会话") }
                }
                else -> LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
                    items(unreachable, key = { "failure/" + it.first.peerId }) { (device, listing) ->
                        DeviceNotice(device.name, listing.failure.orEmpty())
                    }
                    sections.forEach { section ->
                        // A list with nothing but recent conversations needs no heading.
                        if (sections.size > 1 || section.kind != SessionSectionKind.Recent) item(key = "h/" + section.kind) { SectionHeader(section.kind) }
                        items(section.rows, key = { section.kind.name + "/" + it.key }) { row ->
                            SessionItem(row, showDevice = multipleDevices && filter == null) { actions.open(row.peer, row.target()) }
                        }
                    }
                }
            }
        }
    }
    if (starting) NewConversationSheet(startable, preferred = filter, dismiss = { starting = false }) { peer, runtime, cwd ->
        starting = false
        actions.open(peer, ConversationTarget.New(runtime, cwd))
    }
}

@Composable
private fun OverflowMenu(devicesReady: Boolean, actions: SessionsActions) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "更多") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Text("设备") }, enabled = devicesReady, onClick = { open = false; actions.devices() })
            DropdownMenuItem(text = { Text("遥测") }, enabled = devicesReady, onClick = { open = false; actions.telemetry() })
            DropdownMenuItem(text = { Text("关于") }, onClick = { open = false; actions.about() })
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchBar(query: String, change: (String) -> Unit, close: () -> Unit) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    TopAppBar(
        navigationIcon = { IconButton(onClick = close) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "结束搜索") } },
        title = {
            TextField(
                value = query, onValueChange = change, placeholder = { Text("搜索标题、项目或设备") }, singleLine = true,
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent,
                ),
            )
        },
        actions = { if (query.isNotEmpty()) IconButton(onClick = { change("") }) { Icon(Icons.Filled.Close, contentDescription = "清除") } },
    )
}

/** Chips that narrow the list to one device; each shows whether its device is reachable. */
@Composable
private fun DeviceFilter(devices: List<DeviceSummary>, filter: String?, select: (String?) -> Unit) {
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        FilterChip(selected = filter == null, onClick = { select(null) }, label = { Text("全部") })
        devices.forEach { device ->
            FilterChip(
                selected = filter == device.peerId,
                onClick = { select(if (filter == device.peerId) null else device.peerId) },
                label = { Text(device.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                leadingIcon = { StatusDot(device.status.tone, size = 8.dp) },
            )
        }
    }
}

@Composable
private fun SectionHeader(kind: SessionSectionKind) {
    val title = when (kind) {
        SessionSectionKind.Waiting -> "等待回应"
        SessionSectionKind.Pinned -> "置顶"
        SessionSectionKind.Recent -> "最近"
    }
    Text(
        title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

/** A device whose list could not be read; the others stay in the list. */
@Composable
private fun DeviceNotice(device: String, failure: String) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.CloudOff, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        headlineContent = { Text("无法读取“$device”的会话", style = MaterialTheme.typography.bodyMedium) },
        supportingContent = { Text(failure, maxLines = 2, overflow = TextOverflow.Ellipsis) },
    )
    HorizontalDivider()
}

/**
 * One conversation: runtime mark, title and age; below, whatever needs
 * attention, then the project and, when several devices are shown, the device.
 */
@Composable
private fun SessionItem(row: SessionRow, showDevice: Boolean, open: () -> Unit) {
    val listed = row.listed
    ListItem(
        leadingContent = { RuntimeBadge(row.runtime, size = 28.dp) },
        headlineContent = {
            Text(row.title, fontWeight = if (listed.unread) FontWeight.SemiBold else null, maxLines = 1, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SessionStatus(row)
                val meta = listOfNotNull(row.project, row.device.takeIf { showDevice }).ifEmpty { listOfNotNull(row.runtime?.name) }
                Text(meta.joinToString(" · "), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        },
        trailingContent = {
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                listed.conversation.updatedAtUnixMs?.let { Text(relativeTime(it), style = MaterialTheme.typography.labelSmall) }
                if (listed.unread) StatusDot(Tone.Active) else Spacer(Modifier.size(8.dp))
            }
        },
        modifier = Modifier.clickable(onClick = open),
    )
}

/** Leads the second line when the conversation runs, waits or failed; nothing when it is idle. */
@Composable
private fun SessionStatus(row: SessionRow) {
    val (text, tone) = when {
        row.waiting -> "等待回应" to Tone.Attention
        row.running -> "运行中" to Tone.Active
        row.listed.failed -> "失败" to Tone.Negative
        else -> return
    }
    if (tone == Tone.Active) CircularProgressIndicator(Modifier.padding(end = 6.dp).size(12.dp), strokeWidth = 1.5.dp, color = tone.color())
    Text(text, color = tone.color(), maxLines = 1)
    Text(" · ", maxLines = 1)
}

/** Where a new conversation runs: a device, a runtime, and a project folder or the runtime's own default. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NewConversationSheet(
    devices: List<Pair<DeviceSummary, DeviceListing>>,
    preferred: String?,
    dismiss: () -> Unit,
    start: (peer: String, runtime: String, cwd: String?) -> Unit,
) {
    var peer by remember { mutableStateOf(devices.firstOrNull { it.first.peerId == preferred }?.first?.peerId ?: devices.first().first.peerId) }
    val listing = devices.first { it.first.peerId == peer }.second
    var runtime by remember(peer) { mutableStateOf(listing.runtimes.first().id) }
    var project by remember(peer) { mutableStateOf(listing.projects.firstOrNull()) }
    ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = dismiss) {
        Column(Modifier.navigationBarsPadding().padding(bottom = 16.dp)) {
            Text("新会话", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp))
            if (devices.size > 1) {
                SheetLabel("设备")
                Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    devices.forEach { (device, _) ->
                        FilterChip(selected = device.peerId == peer, onClick = { peer = device.peerId }, label = { Text(device.name) }, leadingIcon = { StatusDot(device.status.tone) })
                    }
                }
            }
            SheetLabel("运行时")
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listing.runtimes.forEach { instance ->
                    FilterChip(
                        selected = instance.id == runtime, onClick = { runtime = instance.id },
                        label = { Text(instance.name) }, leadingIcon = { RuntimeBadge(listing.runtimeLabels[instance.id], size = 18.dp) },
                    )
                }
            }
            SheetLabel("项目")
            LazyColumn(Modifier.weight(1f, fill = false)) {
                items(listing.projects, key = { "p/$it" }) { path ->
                    ProjectChoice(listing.projectNames[path] ?: lastPathComponent(path), path, path == project) { project = path }
                }
                item(key = "none") { ProjectChoice("不指定", "使用运行时默认目录", project == null) { project = null } }
            }
            Spacer(Modifier.size(8.dp))
            Button(onClick = { start(peer, runtime, project) }, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp)) { Text("开始") }
        }
    }
}

@Composable
private fun SheetLabel(text: String) {
    Text(text, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 8.dp))
}

@Composable
private fun ProjectChoice(title: String, detail: String, selected: Boolean, select: () -> Unit) {
    ListItem(
        leadingContent = { RadioButton(selected = selected, onClick = null) },
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(detail, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.selectable(selected, onClick = select).padding(horizontal = 8.dp),
    )
}
