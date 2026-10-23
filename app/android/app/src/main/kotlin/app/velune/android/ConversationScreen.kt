package app.velune.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingInteractionAnswer
import app.velune.bindings.BindingInteractionKind
import app.velune.bindings.BindingModelSelection
import app.velune.bindings.BindingRunState
import app.velune.bindings.BindingRuntimeInteraction
import app.velune.bindings.BindingRuntimeInteractionReply

/** What the header says about the conversation besides its title. */
data class ConversationHeader(val title: String, val runtime: RuntimeLabel?, val device: String?, val cwd: String?)

class ConversationActions(
    val back: () -> Unit,
    val send: (String, onSent: () -> Unit) -> Unit,
    val stop: () -> Unit,
    val reply: (interactionId: String, BindingRuntimeInteractionReply) -> Unit,
    val select: (BindingModelSelection?) -> Unit,
    val load: (messageId: String) -> Unit,
)

@Composable
fun ConversationScreen(hub: DeviceHub, peer: String, runtime: RuntimeLabel?, model: ConversationModel, back: () -> Unit) {
    val state by model.state.collectAsState()
    val remote by hub.state.collectAsState()
    WhileVisible(model, resume = model::resume, pause = model::pause)
    val target = model.target
    val header = ConversationHeader(
        title = state.conversation?.title ?: (target as? ConversationTarget.Existing)?.title ?: "新会话",
        runtime = runtime,
        device = remote?.contacts?.firstOrNull { it.peerId == peer }?.name,
        cwd = state.conversation?.cwd ?: (target as? ConversationTarget.New)?.cwd,
    )
    ConversationContent(header, state, remember(model, back) {
        ConversationActions(back, model::send, model::stop, model::reply, model::select, { model.loadPayload(it) })
    })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationContent(header: ConversationHeader, state: ConversationState, actions: ConversationActions) {
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = actions.back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
                title = {
                    Column {
                        Text(header.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            header.runtime?.let { RuntimeBadge(it, Modifier.size(14.dp)) }
                            Text(
                                listOfNotNull(header.runtime?.name, header.device, header.cwd?.let(::lastPathComponent)).joinToString(" · "),
                                style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp),
                            )
                        }
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    state.loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    state.entries.isEmpty() && state.sending == null && state.conversation == null -> EmptyState(
                        Icons.Outlined.ChatBubbleOutline, "新会话",
                        listOfNotNull(header.runtime?.name?.let { "在 $it 上开始" }, "工作目录：" + (header.cwd ?: "运行时默认目录")).joinToString("\n"),
                    )
                    else -> Transcript(state, actions.load, Modifier.fillMaxSize())
                }
            }
            state.failure?.let { Failure(it) }
            state.interactions.firstOrNull()?.let { interaction ->
                InteractionCard(interaction, enabled = !state.busy) { reply -> actions.reply(interaction.id, reply) }
            }
            Composer(state, actions)
        }
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

/** An approval or a question from the runtime, above the input like a pinned message. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun InteractionCard(interaction: BindingRuntimeInteraction, enabled: Boolean, reply: (BindingRuntimeInteractionReply) -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(16.dp),
        border = BorderStroke(1.dp, Tone.Attention.color().copy(alpha = 0.5f)),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            when (val kind = interaction.kind) {
                is BindingInteractionKind.Approval -> {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Outlined.Shield, contentDescription = null, modifier = Modifier.size(20.dp), tint = Tone.Attention.color())
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
                            FilledTonalButton(enabled = enabled, onClick = { reply(BindingRuntimeInteractionReply.Decision(option.id)) }) { Text(option.label) }
                        }
                    }
                }
                is BindingInteractionKind.UserInput -> {
                    val answers = remember(interaction.id) { mutableStateMapOf<String, String>() }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.AutoMirrored.Outlined.HelpOutline, contentDescription = null, modifier = Modifier.size(20.dp), tint = Tone.Attention.color())
                        Text("需要你的回答", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 8.dp))
                    }
                    kind.questions.forEach { question ->
                        Text(question.text, style = MaterialTheme.typography.bodyMedium)
                        if (question.options.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            question.options.forEach { option ->
                                if (answers[question.id] == option.label) Button(onClick = {}) { Text(option.label) }
                                else OutlinedButton(onClick = { answers[question.id] = option.label }) { Text(option.label) }
                            }
                        } else OutlinedTextField(
                            value = answers[question.id].orEmpty(), onValueChange = { answers[question.id] = it },
                            visualTransformation = if (question.secret) PasswordVisualTransformation() else VisualTransformation.None,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(enabled = enabled && kind.questions.all { !answers[it.id].isNullOrBlank() }, onClick = {
                            reply(BindingRuntimeInteractionReply.Answers(kind.questions.map { BindingInteractionAnswer(it.id, listOf(answers[it.id].orEmpty())) }))
                        }) { Text("提交") }
                        TextButton(enabled = enabled, onClick = { reply(BindingRuntimeInteractionReply.Cancel) }) { Text("取消") }
                    }
                }
            }
        }
    }
}

/** The input: text on top, the next turn's model and the send or stop button below, in one rounded field. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Composer(state: ConversationState, actions: ConversationActions) {
    var text by rememberSaveable { mutableStateOf("") }
    var choosing by remember { mutableStateOf(false) }
    val canSend = !state.busy && text.isNotBlank() && state.canSend
    Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 8.dp)) {
        RunStatus(state.runState)
        Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(24.dp), modifier = Modifier.fillMaxWidth()) {
            Column {
                TextField(
                    value = text, onValueChange = { text = it }, placeholder = { Text("给 Agent 发消息") },
                    maxLines = 6, modifier = Modifier.fillMaxWidth(),
                    colors = TextFieldDefaults.colors(
                        focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent, disabledContainerColor = Color.Transparent,
                        focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent, disabledIndicatorColor = Color.Transparent,
                    ),
                )
                Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (state.models.isNotEmpty()) {
                        val name = state.models.firstOrNull { it.selection == state.selection }?.name ?: "运行时默认模型"
                        Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHighest, modifier = Modifier.widthIn(max = 240.dp)) {
                            Row(Modifier.clickable { choosing = true }.padding(start = 10.dp, end = 6.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                                Icon(Icons.Outlined.AutoAwesome, contentDescription = null, modifier = Modifier.size(16.dp))
                                Text(name, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp).weight(1f, fill = false))
                                Icon(Icons.Filled.ArrowDropDown, contentDescription = "选择模型", modifier = Modifier.size(20.dp))
                            }
                        }
                    }
                    Spacer(Modifier.weight(1f))
                    if (state.running && state.canCancel) {
                        FilledTonalIconButton(enabled = !state.busy, onClick = actions.stop, modifier = Modifier.size(40.dp)) {
                            Icon(Icons.Filled.Stop, contentDescription = "停止")
                        }
                    } else {
                        FilledIconButton(
                            enabled = canSend, onClick = { actions.send(text.trim()) { text = "" } }, modifier = Modifier.size(40.dp),
                            colors = IconButtonDefaults.filledIconButtonColors(disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest),
                        ) {
                            if (state.busy && state.sending != null) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            else Icon(Icons.Filled.ArrowUpward, contentDescription = "发送")
                        }
                    }
                }
            }
        }
    }
    if (choosing) ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = { choosing = false }) {
        Text("下一轮的模型", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
        LazyColumn(Modifier.padding(bottom = 24.dp)) {
            item { ModelOption("运行时默认模型", state.selection == null) { choosing = false; actions.select(null) } }
            items(state.models, key = { it.name + "/" + it.selection.hashCode() }) { option ->
                ModelOption(option.name, option.selection == state.selection) { choosing = false; actions.select(option.selection) }
            }
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

/** A line above the input while the conversation is not simply idle. */
@Composable
private fun RunStatus(runState: BindingRunState) {
    val (tone, text) = when (runState) {
        BindingRunState.IDLE -> return
        BindingRunState.RUNNING -> Tone.Active to "运行中"
        BindingRunState.STOPPING -> Tone.Attention to "正在停止"
        BindingRunState.FAILED -> Tone.Negative to "上一轮失败"
        BindingRunState.ELSEWHERE -> Tone.Attention to "正在其他地方运行"
    }
    Row(Modifier.padding(start = 12.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        StatusDot(tone)
        Text(text, style = MaterialTheme.typography.labelMedium, color = tone.color(), modifier = Modifier.padding(start = 6.dp))
    }
}
