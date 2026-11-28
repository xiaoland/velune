package app.velune.android

import app.velune.bindings.conversation.ControlHandoff
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.FormatListBulleted
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Group
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material.icons.outlined.WorkHistory
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredImageSource
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.conversation.AsyncQuestionDelivery
import app.velune.bindings.conversation.EditPlacement
import app.velune.bindings.conversation.EditableMessage
import app.velune.bindings.conversation.InteractionAnswer
import app.velune.bindings.conversation.InteractionKind
import app.velune.bindings.conversation.InteractionQuestion
import app.velune.bindings.conversation.ModelSelection
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteraction
import app.velune.bindings.conversation.RuntimeInteractionReply
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** What the header says about the conversation besides its title. */
data class ConversationHeader(
    val title: String,
    val runtime: RuntimeLabel?,
    val device: String?,
    val cwd: String?,
    /** Non-null only when the connection to the controlled device is not live. */
    val connection: DeviceStatus? = null,
)

class ConversationActions(
    val back: () -> Unit,
    val send: (String, onSent: () -> Unit) -> Unit,
    val stop: () -> Unit,
    val reply: (interactionId: String, RuntimeInteractionReply) -> Unit,
    val select: (ModelSelection?) -> Unit,
    val load: (messageId: String) -> Unit,
    val compact: () -> Unit = {},
    val editDraft: (String) -> Unit = {},
    val removeAttachment: (String) -> Unit = {},
    val setConversationHandoff: (release: Boolean) -> Unit = {},
    val selectEffort: (String?) -> Unit = {},
    val selectPermissionMode: (String?) -> Unit = {},
    val editMessage: (messageId: String, text: String, placement: EditPlacement, expectedRevision: String, onDone: () -> Unit) -> Unit = { _, _, _, _, onDone -> onDone() },
    val forkConversation: (userMessageId: String) -> Unit = {},
    val selectRuntime: (String) -> Unit = {},
    val addAttachments: (List<suspend () -> Pair<String, ByteArray>>) -> Unit = {},
    val refresh: () -> Unit = {},
    val refreshBackgroundTasks: () -> Unit = {},
    val loadBackgroundTaskOutput: (String) -> Unit = {},
    val stopBackgroundTask: (String) -> Unit = {},
    val stopSubAgents: () -> Unit = {},
    val findLinkedConversation: suspend (String) -> ConversationTarget.Existing = { error("设备上找不到此会话") },
    val openConversation: (ConversationTarget.Existing) -> Unit = {},
    val steer: (String) -> Unit = {},
    val queue: (String) -> Unit = {},
    val resumeTurn: () -> Unit = {},
)

@Composable
fun ConversationScreen(
    hub: DeviceHub,
    peer: String,
    listing: DeviceListing,
    model: ConversationModel,
    back: () -> Unit,
    openConversation: (ConversationTarget.Existing) -> Unit = {},
    newSession: NewSessionContext? = null,
) {
    val state by model.state.collectAsState()
    val remote by hub.state.collectAsState()
    WhileVisible(model, resume = model::resume, pause = model::pause)
    val target = model.target
    val contact = remote?.contacts?.firstOrNull { it.peerId == peer }
    val connection = if (remote != null && contact != null) {
        deviceStatus(checkNotNull(remote), contact).takeIf { it.tone != Tone.Positive }
    } else null
    val header = ConversationHeader(
        title = state.conversation?.title?.displayText() ?: (target as? ConversationTarget.Existing)?.title ?: "新会话",
        runtime = listing.runtimeLabels[target.runtimeId],
        device = contact?.name,
        cwd = state.conversation?.cwd ?: (target as? ConversationTarget.New)?.cwd,
        connection = connection,
    )
    ConversationContent(header, state, remember(model, back, openConversation) {
        ConversationActions(
            back = back,
            send = model::send,
            stop = model::stop,
            reply = model::reply,
            select = model::select,
            load = { model.loadPayload(it) },
            compact = model::compact,
            editDraft = model::editDraft,
            removeAttachment = model::removeAttachment,
            setConversationHandoff = model::setConversationHandoff,
            selectEffort = model::selectEffort,
            selectPermissionMode = model::selectPermissionMode,
            editMessage = { id, text, placement, revision, onDone -> model.editMessage(id, text, placement, revision, onDone, openConversation) },
            forkConversation = { id -> model.forkConversation(id, openConversation) },
            selectRuntime = model::selectRuntime,
            addAttachments = model::addAttachments,
            refresh = model::refresh,
            refreshBackgroundTasks = model::refreshBackgroundTasks,
            loadBackgroundTaskOutput = model::loadBackgroundTaskOutput,
            stopBackgroundTask = model::stopBackgroundTask,
            stopSubAgents = model::stopSubAgents,
            findLinkedConversation = model::findLinkedConversation,
            openConversation = openConversation,
            steer = model::steer,
            queue = model::queue,
            resumeTurn = model::resumeTurn,
        )
    },
        newSession = newSession,
        existing = target is ConversationTarget.Existing,
        listing = listing,
    )
}

private enum class ConversationPanel { BACKGROUND_TASKS, COLLABORATION }

private data class HistoryEdit(val message: DeliveredMessage, val eligibility: EditableMessage, val revision: String)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationContent(
    header: ConversationHeader,
    state: ConversationState,
    actions: ConversationActions,
    newSession: NewSessionContext? = null,
    existing: Boolean = state.conversation != null,
    listing: DeviceListing = DeviceListing(),
) {
    val spacing = LocalAppearance.current.spacingScale
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val outline = remember(state.entries) { outlineTurns(state.entries) }
    var showOutline by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf<HistoryEdit?>(null) }
    var forking by remember { mutableStateOf<String?>(null) }
    var panel by remember { mutableStateOf<ConversationPanel?>(null) }
    val subtitle = remember(header) {
        listOfNotNull(header.runtime?.name, header.cwd?.let(::lastPathComponent), header.device).joinToString(" · ")
    }
    val hasLinkedAgents = state.parentConversationId != null || state.subAgents.isNotEmpty() || state.collaborators.isNotEmpty()
    val backgroundTaskCount = state.activeBackgroundTaskCount
    val outlineInMenu = outline.size >= 2 && existing && hasLinkedAgents && backgroundTaskCount > 0
    val hasOverflow = existing || state.canCompact || state.controlHandoff == ControlHandoff.CAN_RELEASE
    val showBackgroundTasks = {
        panel = ConversationPanel.BACKGROUND_TASKS
        actions.refreshBackgroundTasks()
    }
    Scaffold(
        // The composer owns the bottom inset: navigation bars at rest and IME while editing.
        contentWindowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top),
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = { IconButton(onClick = actions.back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
                    title = {
                        Column {
                            Text(header.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (newSession == null && subtitle.isNotEmpty()) {
                                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    },
                    actions = {
                        if (outline.size >= 2 && !outlineInMenu) {
                            IconButton(onClick = { showOutline = true }) {
                                Icon(Icons.AutoMirrored.Filled.FormatListBulleted, contentDescription = "大纲")
                            }
                        }
                        if (existing) {
                            IconButton(onClick = actions.refresh, enabled = !state.refreshing && !state.loading && !state.busy) {
                                if (state.refreshing) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                                else Icon(Icons.Filled.Refresh, contentDescription = "刷新会话")
                            }
                            if (backgroundTaskCount > 0) {
                                IconButton(onClick = showBackgroundTasks) {
                                    BadgedBox(badge = { Badge { Text(backgroundTaskCount.toString()) } }) {
                                        Icon(Icons.Outlined.WorkHistory, contentDescription = "后台任务，$backgroundTaskCount 个运行中")
                                    }
                                }
                            }
                            if (hasLinkedAgents) {
                                IconButton(onClick = { panel = ConversationPanel.COLLABORATION }) {
                                    Icon(Icons.Outlined.Group, contentDescription = "Agent 协作")
                                }
                            }
                        }
                        if (hasOverflow) {
                            var more by remember { mutableStateOf(false) }
                            Box {
                                IconButton(onClick = { more = true }) {
                                    Icon(Icons.Filled.MoreVert, contentDescription = "更多会话操作")
                                }
                                DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                                    if (outlineInMenu) DropdownMenuItem(text = { Text("大纲") }, onClick = {
                                        more = false
                                        showOutline = true
                                    })
                                    if (existing) {
                                        DropdownMenuItem(text = { Text("后台任务") }, onClick = {
                                            more = false
                                            showBackgroundTasks()
                                        })
                                        DropdownMenuItem(text = { Text("Agent 协作") }, onClick = {
                                            more = false
                                            panel = ConversationPanel.COLLABORATION
                                        })
                                    }
                                    if (state.canCompact) DropdownMenuItem(text = { Text("压缩上下文") }, enabled = !state.busy, onClick = {
                                        more = false
                                        actions.compact()
                                    })
                                    if (state.controlHandoff == ControlHandoff.CAN_RELEASE) DropdownMenuItem(text = { Text("释放会话控制权") }, enabled = !state.busy, onClick = {
                                        more = false
                                        actions.setConversationHandoff(true)
                                    })
                                }
                            }
                        }
                    },
                )
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                header.connection?.let { status ->
                    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, modifier = Modifier.fillMaxWidth()) {
                        Row(Modifier.padding(horizontal = 16.dp * spacing, vertical = 6.dp * spacing), verticalAlignment = Alignment.CenterVertically) {
                            StatusDot(status.tone, size = 7.dp)
                            Text(
                                "${header.device ?: "设备"} · ${status.text}",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(start = 8.dp),
                            )
                        }
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                }
            }
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).imePadding()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    state.loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    state.entries.isEmpty() && state.sending == null && state.conversation == null -> Unit
                    else -> Transcript(
                        state, list, actions.load,
                        onEdit = edit@{ message ->
                            val eligibility = state.editable[message.id] ?: return@edit
                            val revision = eligibility.revision ?: return@edit
                            editing = HistoryEdit(message, eligibility, revision)
                        },
                        onFork = { userMessageId -> forking = userMessageId },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
            state.failure?.let { Failure(it) }
            state.interactions.firstOrNull()?.let { interaction ->
                InteractionCard(interaction, enabled = !state.busy, childConversation = state.conversation?.parentConversationId != null) { reply ->
                    actions.reply(interaction.id, reply)
                }
            }
            if (newSession != null) NewConversationContext(
                newSession,
                enabled = !state.busy && !state.uploadingAttachment,
                hasAttachments = state.attachments.isNotEmpty(),
            )
            ConversationComposer(state, actions, listing.runtimes, listing.runtimeLabels)
        }
    }
    when (panel) {
        ConversationPanel.BACKGROUND_TASKS -> BackgroundTasksSheet(
            state, onDismiss = { panel = null }, onRefresh = actions.refreshBackgroundTasks,
            onLoadOutput = actions.loadBackgroundTaskOutput, onStop = actions.stopBackgroundTask,
        )
        ConversationPanel.COLLABORATION -> LinkedConversations(
            state, listing, actions, onDismiss = { panel = null },
        )
        null -> Unit
    }
    if (showOutline) {
        val reversed = state.entries.asReversed()
        val hasWorking = state.running && (state.entries.lastOrNull() as? TranscriptEntry.Work)?.running != true
        val leadingOffset = (if (hasWorking) 1 else 0) + (if (state.sending != null) 1 else 0)
        ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = { showOutline = false }) {
            Text("大纲", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
            LazyColumn(Modifier.navigationBarsPadding().padding(bottom = 16.dp)) {
                items(outline, key = { it.key }) { turn ->
                    ListItem(
                        headlineContent = { Text(turn.preview, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                        trailingContent = turn.timestampUnixMs?.let { ts -> { Text(formatMessageTime(ts), style = MaterialTheme.typography.labelSmall) } },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        modifier = Modifier.clickable {
                            showOutline = false
                            val index = reversed.indexOfFirst { it.key == turn.key }
                            if (index >= 0) scope.launch { list.scrollToItem(leadingOffset + index) }
                        }.padding(horizontal = 8.dp),
                    )
                }
            }
        }
    }
    editing?.let { (message, info, revision) ->
        EditMessageSheet(
            initial = message.editableText(),
            preservedAttachments = message.originalAttachmentNames(),
            editable = info,
            running = state.running,
            busy = state.busy,
            failure = state.failure,
            dismiss = { if (!state.busy) editing = null },
            submit = { updated, placement ->
                actions.editMessage(message.id, updated, placement, revision) { editing = null }
            },
        )
    }
    forking?.let { userMessageId ->
        AlertDialog(
            onDismissRequest = { forking = null },
            title = { Text("从此轮分叉新会话？") },
            text = { Text("新会话将包含截至本轮结束的全部内容，原会话保持不变。") },
            confirmButton = {
                TextButton(enabled = !state.busy, onClick = {
                    forking = null
                    actions.forkConversation(userMessageId)
                }) { Text("创建分叉") }
            },
            dismissButton = { TextButton(onClick = { forking = null }) { Text("取消") } },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditMessageSheet(
    initial: String,
    preservedAttachments: List<String>,
    editable: EditableMessage,
    running: Boolean,
    busy: Boolean,
    failure: String?,
    dismiss: () -> Unit,
    submit: (String, EditPlacement) -> Unit,
) {
    var text by remember(initial) { mutableStateOf(initial) }
    var placement by remember(editable) { mutableStateOf(if (editable.inPlace) EditPlacement.IN_PLACE else EditPlacement.FORK) }
    ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = dismiss) {
        Column(Modifier.navigationBarsPadding().padding(horizontal = 24.dp).padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("编辑消息", style = MaterialTheme.typography.titleLarge)
            if (failure != null) {
                Text(failure, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().heightIn(min = 96.dp, max = 220.dp),
                placeholder = { Text("修改消息内容") },
            )
            if (preservedAttachments.isNotEmpty()) {
                Text(
                    "保留原附件：${preservedAttachments.joinToString("、")}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Column {
                if (editable.inPlace) {
                    val detail = if (running) "将先停止当前轮次，再替换此消息及之后的内容" else "替换此消息及之后的内容"
                    PlacementChoice("在原会话中继续", detail, placement == EditPlacement.IN_PLACE) { placement = EditPlacement.IN_PLACE }
                }
                PlacementChoice("在新会话中继续", "保留原会话，从此处创建分支并发送", placement == EditPlacement.FORK) { placement = EditPlacement.FORK }
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                TextButton(onClick = dismiss) { Text("取消") }
                Button(enabled = !busy && (text.isNotBlank() || preservedAttachments.isNotEmpty()), onClick = { submit(text.trim(), placement) }) {
                    Text(if (placement == EditPlacement.IN_PLACE) "替换并发送" else "分叉并发送")
                }
            }
        }
    }
}

internal fun attachmentDisplayName(index: Int, block: DeliveredBlock): String = when (block) {
    is DeliveredBlock.File -> lastPathComponent(block.path).ifBlank { "原附件" }
    is DeliveredBlock.Image -> when (val source = block.source) {
        is DeliveredImageSource.Path -> lastPathComponent(source.path).ifBlank { "原消息图片 ${index + 1}.${imageExtension(block.mediaType)}" }
        is DeliveredImageSource.Data -> "原消息图片 ${index + 1}.${imageExtension(block.mediaType)}"
    }
    else -> "原附件"
}

private fun DeliveredMessage.originalAttachmentNames(): List<String> =
    blocks.mapIndexedNotNull { index, block ->
        if (block is DeliveredBlock.Image || block is DeliveredBlock.File) attachmentDisplayName(index, block) else null
    }

private fun imageExtension(mediaType: String?): String = when (mediaType?.lowercase()) {
    "image/jpeg" -> "jpg"
    "image/gif" -> "gif"
    "image/webp" -> "webp"
    "image/bmp" -> "bmp"
    "image/heic", "image/heif" -> "heic"
    "image/avif" -> "avif"
    "image/tiff" -> "tiff"
    else -> "png"
}

@Composable
private fun PlacementChoice(title: String, detail: String, selected: Boolean, select: () -> Unit) {
    ListItem(
        leadingContent = { RadioButton(selected = selected, onClick = null) },
        headlineContent = { Text(title) },
        supportingContent = { Text(detail) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.selectable(selected, onClick = select),
    )
}

@Composable
private fun LinkedConversations(
    state: ConversationState,
    listing: DeviceListing,
    actions: ConversationActions,
    onDismiss: () -> Unit,
) {
    var opening by remember { mutableStateOf<String?>(null) }
    var openFailure by remember { mutableStateOf<String?>(null) }
    val running = remember(listing.active) {
        listing.active.values.filter { it.runState == RunState.RUNNING || it.runState == RunState.STOPPING || it.runState == RunState.ELSEWHERE }
            .mapTo(mutableSetOf()) { it.conversationId }
    }
    val waiting = remember(listing.active) {
        listing.active.values.filter { it.pendingInteractions > 0u }.mapTo(mutableSetOf()) { it.conversationId }
    }
    val sessions = remember(listing.conversations) { listing.conversations.map { it.conversation } }
    val open = { target: ConversationTarget.Existing ->
        onDismiss()
        actions.openConversation(target)
    }
    LaunchedEffect(opening) {
        val id = opening ?: return@LaunchedEffect
        try {
            open(actions.findLinkedConversation(id))
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            openFailure = "无法打开关联会话：${error.describe()}"
        } finally {
            opening = null
        }
    }
    CollaborationSheet(
        state = state,
        runtimeId = state.contextRuntimeId,
        runtimeLabels = listing.runtimeLabels,
        sessions = sessions,
        runningIds = running,
        waitingIds = waiting,
        openFailure = openFailure ?: state.collaborationFailure,
        openingConversationId = opening,
        onDismiss = onDismiss,
        onStop = actions.stopSubAgents,
        onOpen = open,
        onOpenUnlisted = { id ->
            openFailure = null
            opening = id
        },
    )
}

@Composable
private fun Failure(text: String) {
    Surface(color = MaterialTheme.colorScheme.errorContainer, contentColor = MaterialTheme.colorScheme.onErrorContainer, shape = RoundedCornerShape(12.dp), modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp)) {
        Row(Modifier.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.ErrorOutline, contentDescription = null, modifier = Modifier.size(18.dp))
            Text(text, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(start = 8.dp))
        }
    }
}

/** An approval or a question from the runtime, anchored above the input. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun InteractionCard(interaction: RuntimeInteraction, enabled: Boolean, childConversation: Boolean, reply: (RuntimeInteractionReply) -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(16.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            when (val kind = interaction.kind) {
                is InteractionKind.Approval -> {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Outlined.Shield, contentDescription = null, modifier = Modifier.size(18.dp), tint = Tone.Attention.color())
                        Text(kind.title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 8.dp))
                    }
                    if (kind.detail.isNotBlank()) Surface(color = MaterialTheme.colorScheme.surfaceContainerLowest, shape = RoundedCornerShape(8.dp)) {
                        SelectionContainer {
                            Text(
                                kind.detail, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace,
                                modifier = Modifier.fillMaxWidth().heightIn(max = 180.dp).verticalScroll(rememberScrollState()).padding(10.dp),
                            )
                        }
                    }
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        kind.options.forEach { option ->
                            FilledTonalButton(enabled = enabled, onClick = { reply(RuntimeInteractionReply.Decision(option.id)) }) { Text(option.label) }
                        }
                    }
                    if (!enabled) Text("正在提交…", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                is InteractionKind.UserInput -> QuestionFields(interaction.id, kind.questions, enabled, readOnly = false, reply = reply)
                is InteractionKind.AsyncUserInput -> QuestionFields(
                    interaction.id, kind.questions, enabled,
                    readOnly = childConversation && kind.delivery == AsyncQuestionDelivery.CONVERSATION_INPUT,
                    reply = reply,
                )
            }
        }
    }
}

private class QuestionDraft(questions: List<InteractionQuestion>) {
    val choices = mutableStateMapOf<String, Set<String>>()
    val text = mutableStateMapOf<String, String>()
    val other = mutableStateMapOf<String, Boolean>()

    init {
        for (question in questions) {
            val value = question.defaultText ?: continue
            when {
                question.options.isEmpty() -> text[question.id] = value
                question.options.any { it.id == value } -> choices[question.id] = setOf(value)
                question.allowOther -> {
                    text[question.id] = value
                    other[question.id] = true
                }
            }
        }
    }

    fun values(question: InteractionQuestion): List<String> {
        val options = choices[question.id].orEmpty()
        val useText = question.options.isEmpty() || other[question.id] == true
        return question.options.filter { it.id in options }.map { it.id } +
            listOfNotNull(text[question.id]?.takeIf { useText && it.isNotBlank() })
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun QuestionFields(id: String, questions: List<InteractionQuestion>, enabled: Boolean, readOnly: Boolean, reply: (RuntimeInteractionReply) -> Unit) {
    val draft = remember(id) { QuestionDraft(questions) }
    val canAnswer = enabled && !readOnly
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.AutoMirrored.Outlined.HelpOutline, contentDescription = null, modifier = Modifier.size(18.dp), tint = Tone.Attention.color())
        Text("需要你的回答", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 8.dp))
    }
    if (readOnly) Text("请回到父会话回答这个问题。", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Column(Modifier.heightIn(max = 240.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        questions.forEachIndexed { index, question ->
            if (index > 0) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
            val selected = draft.choices[question.id].orEmpty()
            val hasOther = draft.other[question.id] == true
            if (questions.size > 1) {
                Text("问题 ${index + 1} / ${questions.size}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(question.text, style = MaterialTheme.typography.bodyMedium)
            if (question.options.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                question.options.forEach { option ->
                    val change = {
                        draft.choices[question.id] = when {
                            !question.multiple -> setOf(option.id)
                            option.id in selected -> selected - option.id
                            else -> selected + option.id
                        }
                        if (!question.multiple) draft.other[question.id] = false
                    }
                    if (option.id in selected) Button(onClick = change, enabled = canAnswer) { Text(option.label) }
                    else OutlinedButton(onClick = change, enabled = canAnswer) { Text(option.label) }
                }
                if (question.allowOther && !readOnly) OutlinedButton(enabled = enabled, onClick = {
                    draft.other[question.id] = !hasOther
                    if (!question.multiple && !hasOther) draft.choices.remove(question.id)
                }) { Text("其他") }
            }
            if (!readOnly && (question.options.isEmpty() || hasOther)) OutlinedTextField(
                value = draft.text[question.id].orEmpty(), onValueChange = { draft.text[question.id] = it },
                visualTransformation = if (question.secret) PasswordVisualTransformation() else VisualTransformation.None,
                modifier = Modifier.fillMaxWidth(), enabled = enabled, singleLine = !question.multiline,
                maxLines = if (question.multiline) 5 else 1,
            )
        }
    }
    if (!readOnly) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Button(enabled = enabled && questions.isNotEmpty() && questions.all { draft.values(it).isNotEmpty() }, onClick = {
            reply(RuntimeInteractionReply.Answers(questions.map { InteractionAnswer(it.id, draft.values(it)) }))
        }) { Text("提交") }
        TextButton(enabled = enabled, onClick = { reply(RuntimeInteractionReply.Cancel) }) { Text("取消") }
    }
}
