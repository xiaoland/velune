package app.velune.android

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingInteractionAnswer
import app.velune.bindings.BindingInteractionKind
import app.velune.bindings.BindingMessage
import app.velune.bindings.BindingMessageBlock
import app.velune.bindings.BindingMessagePayload
import app.velune.bindings.BindingMessageRole
import app.velune.bindings.BindingRunState
import app.velune.bindings.BindingRuntimeInteraction
import app.velune.bindings.BindingRuntimeInteractionReply
import app.velune.bindings.BindingToolState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConversationScreen(hub: DeviceHub, peer: String, model: ConversationModel, back: () -> Unit) {
    val state by model.state.collectAsState()
    val remote by hub.state.collectAsState()
    WhileVisible(model, resume = model::resume, pause = model::pause)
    val device = remote?.contacts?.firstOrNull { it.peerId == peer }?.name
    val title = state.snapshot?.conversation?.title ?: (model.target as? ConversationTarget.Existing)?.title ?: "新会话"
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
                title = {
                    Column {
                        Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        device?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                if (state.loading) CircularProgressIndicator(Modifier.align(Alignment.Center))
                else Transcript(state, model)
            }
            state.failure?.let {
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
            }
            state.snapshot?.pendingInteractions?.firstOrNull()?.let { interaction ->
                InteractionCard(interaction, enabled = !state.busy) { reply -> model.reply(interaction.id, reply) }
            }
            HorizontalDivider()
            Composer(state, model)
        }
    }
}

@Composable
private fun Transcript(state: ConversationState, model: ConversationModel) {
    val list = rememberLazyListState()
    val expanded = remember { mutableStateMapOf<String, Boolean>() }
    // Follow the end while the reply grows, as long as the user has not scrolled away.
    val last = state.messages.lastOrNull()
    LaunchedEffect(state.messages.size, last?.stamp) {
        if (state.messages.isNotEmpty() && (!list.canScrollForward || list.firstVisibleItemIndex == 0)) list.scrollToItem(state.messages.size - 1)
    }
    if (state.messages.isEmpty()) {
        Message(if (state.snapshot == null) "输入第一条消息开始会话。" else "还没有消息。")
        return
    }
    LazyColumn(state = list, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.fillMaxSize()) {
        items(state.messages, key = { it.id }) { message ->
            MessageView(state.payloads[message.id] ?: message, expanded, load = { model.loadPayload(message.id) })
        }
        if (state.snapshot?.runState == BindingRunState.RUNNING) item(key = "running") {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) { CircularProgressIndicator(Modifier.padding(4.dp), strokeWidth = 2.dp) }
        }
    }
}

/** The user's messages on the right, the model's on the left, the system's and the Harness's in the middle. */
@Composable
private fun MessageView(message: BindingMessage, expanded: MutableMap<String, Boolean>, load: () -> Unit) {
    when (message.role) {
        BindingMessageRole.USER -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Surface(color = MaterialTheme.colorScheme.secondaryContainer, shape = RoundedCornerShape(18.dp), modifier = Modifier.widthIn(max = 320.dp)) {
                Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    message.blocks.forEach { block -> Block(message.id, block, expanded, load) }
                }
            }
        }
        BindingMessageRole.ASSISTANT -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            message.blocks.forEach { block -> Block(message.id, block, expanded, load) }
        }
        BindingMessageRole.TOOL, BindingMessageRole.SYSTEM -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            message.blocks.forEach { block -> Block(message.id, block, expanded, load) }
        }
    }
}

@Composable
private fun Block(messageId: String, block: BindingMessageBlock, expanded: MutableMap<String, Boolean>, load: () -> Unit) {
    when (block) {
        is BindingMessageBlock.Text -> SelectionContainer { Text(block.text, style = MaterialTheme.typography.bodyLarge) }
        is BindingMessageBlock.Notice -> Text(
            block.text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth(),
        )
        is BindingMessageBlock.Reasoning -> Disclosure("$messageId/reasoning", block.summary.ifBlank { "思考" }, block.text, expanded, load)
        is BindingMessageBlock.Tool -> Disclosure(
            "$messageId/tool/${block.toolId ?: block.title}",
            block.title + when (block.state) {
                BindingToolState.PENDING -> " · 等待"
                BindingToolState.RUNNING -> " · 运行中"
                BindingToolState.COMPLETED -> ""
                BindingToolState.FAILED -> " · 失败"
            },
            block.output, expanded, load,
        )
        is BindingMessageBlock.Image -> Text("［图片］", color = MaterialTheme.colorScheme.onSurfaceVariant)
        is BindingMessageBlock.File -> Text("［文件］" + block.path.substringAfterLast('/'), color = MaterialTheme.colorScheme.onSurfaceVariant)
        is BindingMessageBlock.AgentSender -> Text("来自 ${block.runtimeName} · ${block.title}", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Reasoning and tool output stay folded; large ones are read when unfolded. */
@Composable
private fun Disclosure(key: String, title: String, payload: BindingMessagePayload?, expanded: MutableMap<String, Boolean>, load: () -> Unit) {
    val open = expanded[key] == true
    Column(Modifier.fillMaxWidth().animateContentSize()) {
        Text(
            (if (open) "▾ " else "▸ ") + title,
            style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.clickable {
                expanded[key] = !open
                if (!open && payload is BindingMessagePayload.Deferred) load()
            }.padding(vertical = 4.dp),
        )
        if (open) when (payload) {
            is BindingMessagePayload.Inline -> SelectionContainer {
                Text(
                    payload.text, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace,
                    modifier = Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp)).padding(10.dp),
                )
            }
            is BindingMessagePayload.Deferred -> CircularProgressIndicator(Modifier.padding(8.dp), strokeWidth = 2.dp)
            null -> {}
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun InteractionCard(interaction: BindingRuntimeInteraction, enabled: Boolean, reply: (BindingRuntimeInteractionReply) -> Unit) {
    ElevatedCard(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp)) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            when (val kind = interaction.kind) {
                is BindingInteractionKind.Approval -> {
                    Text(kind.title, style = MaterialTheme.typography.titleMedium)
                    if (kind.detail.isNotBlank()) SelectionContainer {
                        Text(kind.detail, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, maxLines = 12, overflow = TextOverflow.Ellipsis)
                    }
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        kind.options.forEach { option ->
                            FilledTonalButton(enabled = enabled, onClick = { reply(BindingRuntimeInteractionReply.Decision(option.id)) }) { Text(option.label) }
                        }
                    }
                }
                is BindingInteractionKind.UserInput -> {
                    val answers = remember(interaction.id) { mutableStateMapOf<String, String>() }
                    kind.questions.forEach { question ->
                        Text(question.text, style = MaterialTheme.typography.titleSmall)
                        if (question.options.isNotEmpty()) FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            question.options.forEach { option ->
                                val chosen = answers[question.id] == option.label
                                if (chosen) Button(onClick = {}) { Text(option.label) }
                                else OutlinedButton(onClick = { answers[question.id] = option.label }) { Text(option.label) }
                            }
                        } else OutlinedTextField(
                            value = answers[question.id].orEmpty(), onValueChange = { answers[question.id] = it },
                            visualTransformation = if (question.secret) PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
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

@Composable
private fun Composer(state: ConversationState, model: ConversationModel) {
    var text by rememberSaveable(model) { mutableStateOf("") }
    var choosing by remember { mutableStateOf(false) }
    val canSend = !state.busy && text.isNotBlank() && (state.snapshot?.actions?.canSend ?: true)
    Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp)) {
        if (state.models.isNotEmpty()) Box {
            AssistChip(
                onClick = { choosing = true },
                label = { Text(state.models.firstOrNull { it.selection == state.selection }?.name ?: "运行时默认模型", maxLines = 1) },
                trailingIcon = { Icon(Icons.Filled.ArrowDropDown, contentDescription = null) },
            )
            DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
                DropdownMenuItem(text = { Text("运行时默认模型") }, onClick = { choosing = false; model.select(null) })
                state.models.forEach { option ->
                    DropdownMenuItem(text = { Text(option.name) }, onClick = { choosing = false; model.select(option.selection) })
                }
            }
        }
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = text, onValueChange = { text = it }, placeholder = { Text("输入消息") },
                maxLines = 6, modifier = Modifier.weight(1f),
            )
            if (state.running && state.snapshot?.actions?.canCancel == true) {
                FilledTonalButton(enabled = !state.busy, onClick = model::stop) { Text("停止") }
            } else {
                FilledIconButton(enabled = canSend, onClick = { model.send(text.trim()) { text = "" } }) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "发送")
                }
            }
        }
    }
}
