package app.velune.android

import app.velune.bindings.conversation.ControlHandoff
import app.velune.bindings.conversation.InputAccess
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.FormatListBulleted
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.InputChip
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
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
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
    val editMessage: (messageId: String, text: String, placement: EditPlacement) -> Unit = { _, _, _ -> },
    val forkConversation: (messageId: String) -> Unit = {},
)

@Composable
fun ConversationScreen(
    hub: DeviceHub,
    peer: String,
    runtime: RuntimeLabel?,
    model: ConversationModel,
    back: () -> Unit,
    openConversation: (ConversationTarget.Existing) -> Unit = {},
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
        runtime = runtime,
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
            editMessage = { id, text, placement -> model.editMessage(id, text, placement, openConversation) },
            forkConversation = { id -> model.forkConversation(id, openConversation) },
        )
    })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationContent(header: ConversationHeader, state: ConversationState, actions: ConversationActions) {
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val outline = remember(state.entries) { outlineTurns(state.entries) }
    var showOutline by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf<DeliveredMessage?>(null) }
    var forking by remember { mutableStateOf<DeliveredMessage?>(null) }
    val subtitle = remember(header) {
        listOfNotNull(header.runtime?.name, header.cwd?.let(::lastPathComponent), header.device).joinToString(" · ")
    }
    val hasOverflow = state.canCompact || state.controlHandoff == ControlHandoff.CAN_RELEASE
    Scaffold(
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = { IconButton(onClick = actions.back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
                    title = {
                        Column {
                            Text(header.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (subtitle.isNotEmpty()) {
                                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    },
                    actions = {
                        if (outline.size >= 2) {
                            IconButton(onClick = { showOutline = true }) {
                                Icon(Icons.AutoMirrored.Filled.FormatListBulleted, contentDescription = "大纲")
                            }
                        }
                        if (hasOverflow) {
                            var more by remember { mutableStateOf(false) }
                            Box {
                                IconButton(onClick = { more = true }, enabled = !state.busy) {
                                    Icon(Icons.Filled.MoreVert, contentDescription = "更多会话操作")
                                }
                                DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
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
                        Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
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
        Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    state.loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    state.entries.isEmpty() && state.sending == null && state.conversation == null -> NewConversationContext(header)
                    else -> Transcript(
                        state, list, actions.load,
                        onEdit = { editing = it },
                        onFork = { forking = it },
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
            Composer(state, actions)
        }
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
    editing?.let { message ->
        val info = state.editable[message.id]
        if (info != null) EditMessageSheet(
            initial = message.copyableText(),
            editable = info,
            running = state.running,
            busy = state.busy,
            dismiss = { editing = null },
            submit = { updated, placement ->
                editing = null
                actions.editMessage(message.id, updated, placement)
            },
        ) else editing = null
    }
    forking?.let { message ->
        AlertDialog(
            onDismissRequest = { forking = null },
            title = { Text("从此轮分叉新会话？") },
            text = { Text("新会话将包含截至本轮结束的全部内容，原会话保持不变。") },
            confirmButton = {
                TextButton(enabled = !state.busy, onClick = {
                    forking = null
                    actions.forkConversation(message.id)
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
    editable: EditableMessage,
    running: Boolean,
    busy: Boolean,
    dismiss: () -> Unit,
    submit: (String, EditPlacement) -> Unit,
) {
    var text by remember(initial) { mutableStateOf(initial) }
    var placement by remember(editable) { mutableStateOf(if (editable.inPlace) EditPlacement.IN_PLACE else EditPlacement.FORK) }
    ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = dismiss) {
        Column(Modifier.navigationBarsPadding().padding(horizontal = 24.dp).padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("编辑消息", style = MaterialTheme.typography.titleLarge)
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                modifier = Modifier.fillMaxWidth().heightIn(min = 96.dp, max = 220.dp),
                placeholder = { Text("修改消息内容") },
            )
            Column {
                if (editable.inPlace) {
                    val detail = if (running) "将先停止当前轮次，再替换此消息及之后的内容" else "替换此消息及之后的内容"
                    PlacementChoice("在原会话中继续", detail, placement == EditPlacement.IN_PLACE) { placement = EditPlacement.IN_PLACE }
                }
                PlacementChoice("在新会话中继续", "保留原会话，从此处创建分支并发送", placement == EditPlacement.FORK) { placement = EditPlacement.FORK }
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                TextButton(onClick = dismiss) { Text("取消") }
                Button(enabled = !busy && text.isNotBlank(), onClick = { submit(text.trim(), placement) }) {
                    Text(if (placement == EditPlacement.IN_PLACE) "替换并发送" else "分叉并发送")
                }
            }
        }
    }
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

/** Quiet context for an empty new conversation so the composer stays the focus. */
@Composable
private fun NewConversationContext(header: ConversationHeader) {
    val targetLine = listOfNotNull(header.device, header.runtime?.name).joinToString(" · ").ifEmpty { "新会话" }
    Column(
        Modifier.fillMaxSize().padding(horizontal = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(targetLine, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurface)
        Text(
            header.cwd ?: "运行时默认目录",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
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

/** The input: text on top, the next turn's model/permission/effort and send/stop below, in one rounded container. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun Composer(state: ConversationState, actions: ConversationActions) {
    val text = state.draft
    var choosingModel by remember { mutableStateOf(false) }
    var choosingConfig by remember { mutableStateOf(false) }
    val canSend = !state.busy && state.composerReady && state.composerFailure == null && (text.isNotBlank() || state.attachments.isNotEmpty()) && state.canSend
    val hasExecutionConfig = state.permissionModes.isNotEmpty() || state.efforts.isNotEmpty()
    val defaultModelLabel = if (state.hasCurrent) {
        state.currentModelName?.let { "沿用当前（$it）" } ?: "沿用当前"
    } else "运行时默认"
    val shownModel = state.models.firstOrNull { it.selection == state.selection }?.name ?: state.currentModelName ?: "默认模型"
    val activePermission = state.permissionModes.firstOrNull { it.id == state.permissionMode } ?: state.currentPermissionMode
    val activeEffort = state.effort ?: state.currentEffort
    val configSummary = listOfNotNull(activePermission?.label, activeEffort).joinToString(" · ").ifEmpty { "权限与推理" }
    Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        ComposerStatus(state, actions)
        if (state.attachments.isNotEmpty()) {
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                state.attachments.forEach { attachment ->
                    InputChip(
                        selected = false,
                        onClick = { actions.removeAttachment(attachment.id) },
                        label = { Text(attachment.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        trailingIcon = { Icon(Icons.Filled.Close, contentDescription = "移除附件", modifier = Modifier.size(16.dp)) },
                    )
                }
            }
        }
        Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(24.dp), modifier = Modifier.fillMaxWidth()) {
            Column {
                val bodyStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface)
                Box(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 6.dp), contentAlignment = Alignment.CenterStart) {
                    if (text.isEmpty()) Text("给 Agent 发消息", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    BasicTextField(
                        value = text,
                        onValueChange = actions.editDraft,
                        readOnly = state.inputAccess == InputAccess.READ_ONLY,
                        textStyle = bodyStyle,
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        maxLines = 6,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Row(
                    Modifier.fillMaxWidth().padding(start = 10.dp, end = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Row(
                        Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        if (state.models.isNotEmpty()) {
                            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                                Row(
                                    Modifier.clickable(enabled = state.composerReady && !state.busy) { choosingModel = true }.padding(start = 12.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text(shownModel, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                                    Icon(Icons.Filled.ArrowDropDown, contentDescription = "选择模型", modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                        if (hasExecutionConfig) {
                            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                                Row(
                                    Modifier.clickable(enabled = state.composerReady && !state.busy) { choosingConfig = true }.padding(start = 10.dp, end = 10.dp, top = 6.dp, bottom = 6.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Icon(Icons.Outlined.Tune, contentDescription = null, modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Text(configSummary, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, modifier = Modifier.padding(start = 5.dp))
                                }
                            }
                        }
                    }
                    if (state.running && state.canCancel) {
                        FilledTonalIconButton(enabled = !state.busy, onClick = actions.stop, modifier = Modifier.size(40.dp)) {
                            Icon(Icons.Filled.Stop, contentDescription = "停止")
                        }
                    } else {
                        FilledIconButton(
                            enabled = canSend, onClick = { actions.send(text.trim()) {} }, modifier = Modifier.size(40.dp),
                            colors = IconButtonDefaults.filledIconButtonColors(
                                containerColor = MaterialTheme.colorScheme.primary,
                                contentColor = MaterialTheme.colorScheme.onPrimary,
                                disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            ),
                        ) {
                            if (state.busy && state.sending != null) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
                            else Icon(Icons.Filled.ArrowUpward, contentDescription = "发送")
                        }
                    }
                }
            }
        }
    }
    if (choosingModel) ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = { choosingModel = false }) {
        Text("下一轮的模型", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
        LazyColumn(Modifier.navigationBarsPadding().padding(bottom = 16.dp)) {
            item { ModelOption(defaultModelLabel, state.selection == null) { choosingModel = false; actions.select(null) } }
            items(state.models, key = { it.name + "/" + it.selection.hashCode() }) { option ->
                ModelOption(option.name, option.selection == state.selection) { choosingModel = false; actions.select(option.selection) }
            }
        }
    }
    if (choosingConfig) ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = { choosingConfig = false }) {
        Column(Modifier.navigationBarsPadding().verticalScroll(rememberScrollState()).padding(bottom = 20.dp)) {
            Text("执行配置", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
            if (state.efforts.isNotEmpty()) {
                val defaultEffort = if (state.hasCurrent) {
                    state.currentEffort?.let { "沿用当前（$it）" } ?: "沿用当前"
                } else "运行时默认"
                Text("推理强度", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 8.dp, bottom = 8.dp))
                FlowRow(Modifier.padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(selected = state.effort == null, onClick = { actions.selectEffort(null) }, label = { Text(defaultEffort) })
                    state.efforts.forEach { level ->
                        FilterChip(selected = state.effort == level, onClick = { actions.selectEffort(level) }, label = { Text(level) })
                    }
                }
            }
            if (state.permissionModes.isNotEmpty()) {
                val defaultPermission = if (state.hasCurrent) {
                    state.currentPermissionMode?.label?.let { "沿用当前（$it）" } ?: "沿用当前"
                } else "运行时默认"
                Text("权限模式", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 4.dp))
                PermissionOption(defaultPermission, null, state.permissionMode == null) {
                    actions.selectPermissionMode(null)
                    choosingConfig = false
                }
                state.permissionModes.forEach { mode ->
                    PermissionOption(mode.label, mode.detail.takeIf { it.isNotBlank() }, state.permissionMode == mode.id) {
                        actions.selectPermissionMode(mode.id)
                        choosingConfig = false
                    }
                }
            }
        }
    }
}

@Composable
private fun PermissionOption(title: String, detail: String?, selected: Boolean, choose: () -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = detail?.let { text -> { Text(text) } },
        trailingContent = { if (selected) Icon(Icons.Filled.Check, contentDescription = "当前选择", tint = MaterialTheme.colorScheme.primary) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClick = choose).padding(horizontal = 8.dp),
    )
}

/** Quiet status lines above the composer, shown only when there is something actionable or blocking. */
@Composable
private fun ComposerStatus(state: ConversationState, actions: ConversationActions) {
    val lockMessage = when (state.inputAccess) {
        InputAccess.EDITABLE -> null
        InputAccess.CHECKING_CONTROL -> "正在检查会话控制权…"
        InputAccess.EXTERNAL_CONTROLLER -> "另一个客户端正在控制此会话，草稿可继续编辑，暂不能发送。"
        InputAccess.CONTROL_UNAVAILABLE -> "暂时无法确认会话控制权，正在重试。"
        InputAccess.EXTERNAL_EXECUTION -> "此会话正在其他客户端执行。"
        InputAccess.READ_ONLY -> "此会话当前只可阅读。"
    }
    when (state.controlHandoff) {
        ControlHandoff.RELEASED, ControlHandoff.RELEASING -> Row(Modifier.padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            val releasing = state.controlHandoff == ControlHandoff.RELEASING
            Text(if (releasing) "正在交还控制权…" else "控制权已交还", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            TextButton(onClick = { actions.setConversationHandoff(false) }, enabled = !state.busy) {
                Text(if (releasing) "取消交接" else "恢复 Velune 执行")
            }
        }
        ControlHandoff.CAN_RELEASE, null -> lockMessage?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 8.dp))
        }
    }
    if (!state.composerReady && state.composerFailure == null) {
        Text("正在恢复输入…", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 8.dp))
    }
    state.composerFailure?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 8.dp))
    }
    val runNotice = when (state.runState) {
        RunState.IDLE, RunState.RUNNING -> null
        RunState.STOPPING -> Tone.Attention to "正在停止"
        RunState.FAILED -> Tone.Negative to "上一轮失败"
        RunState.ELSEWHERE -> Tone.Attention to "正在其他地方运行"
    }
    runNotice?.let { (tone, label) ->
        Row(Modifier.padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            StatusDot(tone, size = 7.dp)
            Text(label, style = MaterialTheme.typography.labelMedium, color = tone.color(), modifier = Modifier.padding(start = 6.dp))
        }
    }
}

@Composable
private fun ModelOption(name: String, selected: Boolean, choose: () -> Unit) {
    ListItem(
        headlineContent = { Text(name) },
        trailingContent = { if (selected) Icon(Icons.Filled.Check, contentDescription = "当前选择", tint = MaterialTheme.colorScheme.primary) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClick = choose).padding(horizontal = 8.dp),
    )
}
