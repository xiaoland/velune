package app.velune.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import app.velune.bindings.*
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val model: RemoteViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                model.foreground(true)
                try { awaitCancellation() } finally { model.foreground(false) }
            }
        }
        setContent {
            val state by model.state.collectAsStateWithLifecycle()
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                VeluneScreen(state, model)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun VeluneScreen(state: RemoteUiState, model: RemoteViewModel) {
    var creating by remember { mutableStateOf(false) }
    var endingQuery by remember { mutableStateOf(false) }
    val snapshot = state.snapshot
    BackHandler(enabled = snapshot != null) { model.showConversations() }
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(snapshot?.conversation?.title ?: state.connected?.name ?: "Velune", maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (state.connected != null) Text(state.connection, style = MaterialTheme.typography.labelSmall)
                    }
                },
                navigationIcon = {
                    if (snapshot != null) IconButton(onClick = model::showConversations) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回会话列表")
                    }
                },
                actions = {
                    if (state.connected != null) {
                        IconButton(onClick = model::refresh, enabled = !state.busy) { Icon(Icons.Default.Refresh, "刷新") }
                        IconButton(onClick = { creating = true }, enabled = !state.busy && state.pending == null && state.runtimeId != null) { Icon(Icons.Default.Add, "新建会话") }
                        IconButton(onClick = model::disconnect, enabled = !state.busy) { Icon(Icons.Default.Close, "断开连接") }
                    }
                },
            )
        },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            if (state.starting || state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            state.error?.let { Feedback(it, true, model::clearError) }
            state.info?.let { Feedback(it, false, model::clearError) }
            state.pending?.let { pending ->
                PendingCard(pending, state.pendingStatus, !state.busy && state.connected?.endpointId == pending.endpointId,
                    model::queryPending, { endingQuery = true })
            }
            when {
                state.connected == null -> DevicesScreen(state, model)
                snapshot == null -> ConversationsScreen(state, model) { creating = true }
                else -> ChatScreen(state, model)
            }
        }
    }
    if (creating) NewConversationDialog(state, model, onDismiss = { creating = false })
    if (endingQuery) AlertDialog(
        onDismissRequest = { endingQuery = false },
        title = { Text("结束查询？") },
        text = { Text("已经接纳的操作仍可能在执行。请先检查会话；结束查询不会取消或重发该操作。") },
        confirmButton = { TextButton(onClick = { endingQuery = false; model.stopTracking() }) { Text("结束查询") } },
        dismissButton = { TextButton(onClick = { endingQuery = false }) { Text("保留查询") } },
    )
}

@Composable
private fun Feedback(text: String, failure: Boolean, dismiss: () -> Unit) {
    Row(Modifier.fillMaxWidth().background(if (failure) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.secondaryContainer).padding(start = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
        TextButton(onClick = dismiss) { Text("关闭") }
    }
}

@Composable
private fun PendingCard(pending: PendingOperation, status: String, canQuery: Boolean, query: () -> Unit, stop: () -> Unit) {
    OutlinedCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text("待确认：${pending.label}", style = MaterialTheme.typography.titleSmall)
            Text(status.ifEmpty { "连接操作所属设备后，查询原请求。" }, style = MaterialTheme.typography.bodySmall)
            SelectionContainer { Text("请求 ${pending.requestId.client.take(8)} / ${pending.requestId.sequence}", style = MaterialTheme.typography.labelSmall) }
            Row {
                TextButton(onClick = query, enabled = canQuery) { Text("查询结果") }
                TextButton(onClick = stop) { Text("结束查询…") }
            }
        }
    }
}

@Suppress("DEPRECATION")
@Composable
private fun DevicesScreen(state: RemoteUiState, model: RemoteViewModel) {
    var invitation by remember { mutableStateOf("") }
    var invitationVisible by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        item {
            Text("连接执行设备", style = MaterialTheme.typography.headlineSmall)
            Text("在 Mac 的远程控制入口创建一次性邀请，再粘贴到这里。设备配对后可重新连接。", style = MaterialTheme.typography.bodyMedium)
        }
        item {
            OutlinedTextField(
                invitation, { invitation = it }, Modifier.fillMaxWidth(), label = { Text("一次性邀请") },
                minLines = 2, maxLines = 4,
                visualTransformation = if (invitationVisible) VisualTransformation.None else PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                trailingIcon = {
                    TextButton(onClick = { invitationVisible = !invitationVisible }) {
                        Text(if (invitationVisible) "隐藏邀请" else "显示邀请")
                    }
                },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { invitation = clipboard.getText()?.text.orEmpty() }, enabled = !state.busy) { Text("粘贴邀请") }
                Button(onClick = { model.pair(invitation) { invitation = ""; invitationVisible = false } }, enabled = !state.starting && !state.busy && invitation.isNotBlank()) { Text("配对并连接") }
            }
            Text("当前使用 iroh 公共开发连接服务；这不是已运营的 Velune 公网服务。", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (state.devices.isNotEmpty()) item { Text("已配对设备", style = MaterialTheme.typography.titleMedium) }
        items(state.devices, key = { it.endpointId }) { descriptor ->
            OutlinedCard(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text(descriptor.name, style = MaterialTheme.typography.titleMedium)
                    Text(descriptor.endpointId.take(16), style = MaterialTheme.typography.labelSmall)
                    Row {
                        Button(onClick = { model.connect(descriptor) }, enabled = !state.busy && !state.starting) { Text("连接") }
                        TextButton(onClick = { model.forget(descriptor) }, enabled = !state.busy && state.pending?.endpointId != descriptor.endpointId) { Text("移除此记录") }
                    }
                }
            }
        }
        item { Text("断开或进入后台不会停止执行设备上的任务。", style = MaterialTheme.typography.bodySmall) }
    }
}

@Composable
private fun ConversationsScreen(state: RemoteUiState, model: RemoteViewModel, create: () -> Unit) {
    val conversations = state.configuration?.conversations.orEmpty()
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(vertical = 8.dp)) {
        if (conversations.isEmpty()) item {
            Column(Modifier.padding(24.dp)) {
                Text("还没有会话", style = MaterialTheme.typography.titleLarge)
                Text("在执行设备已经配置的运行时上创建会话。")
                Button(onClick = create, enabled = state.runtimeId != null && state.pending == null && !state.busy) { Text("新建会话") }
            }
        }
        items(conversations, key = { "${it.conversation.runtimeId}/${it.conversation.id}" }) { listed ->
            val conversation = listed.conversation
            ListItem(
                headlineContent = { Text(conversation.title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                supportingContent = { Text(listOfNotNull(state.configuration?.runtimeInstances?.firstOrNull { it.id == conversation.runtimeId }?.name, conversation.cwd).joinToString(" · "), maxLines = 2, overflow = TextOverflow.Ellipsis) },
                modifier = Modifier.clickable(enabled = !state.busy) { model.openConversation(conversation) },
            )
            HorizontalDivider(Modifier.padding(horizontal = 16.dp))
        }
        state.configuration?.historyFailures?.takeIf { it.isNotEmpty() }?.let {
            item { Text("部分原生历史未能读取，可在执行设备检查运行时状态。", Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error) }
        }
    }
}

@Composable
private fun RuntimeAndModel(state: RemoteUiState, model: RemoteViewModel) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val runtimes = state.configuration?.runtimeInstances.orEmpty().filter { it.enabled }
        ChoiceMenu(
            label = runtimes.firstOrNull { it.id == state.runtimeId }?.name ?: "选择运行时",
            choices = runtimes.map { it.name to { model.selectRuntime(it.id) } },
            enabled = !state.busy && state.pending == null,
            modifier = Modifier.weight(1f),
        )
        ChoiceMenu(
            label = state.models.firstOrNull { it.selection == state.selectedModel }?.name ?: "运行时默认模型",
            choices = listOf("运行时默认模型" to { model.selectModel(BindingModelSelection.RuntimeDefault) }) + state.models.map { it.name to { model.selectModel(it.selection) } },
            enabled = !state.busy && state.pending == null,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
private fun ChoiceMenu(label: String, choices: List<Pair<String, () -> Unit>>, enabled: Boolean, modifier: Modifier = Modifier) {
    var expanded by remember { mutableStateOf(false) }
    Box(modifier) {
        OutlinedButton(onClick = { expanded = true }, enabled = enabled, modifier = Modifier.fillMaxWidth()) { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        DropdownMenu(expanded, onDismissRequest = { expanded = false }) {
            choices.forEach { (name, action) -> DropdownMenuItem(text = { Text(name) }, onClick = { expanded = false; action() }) }
        }
    }
}

@Composable
private fun NewConversationDialog(state: RemoteUiState, model: RemoteViewModel, onDismiss: () -> Unit) {
    var cwd by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("新建会话") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                RuntimeAndModel(state, model)
                OutlinedTextField(cwd, { cwd = it }, label = { Text("执行设备工作目录（可留空）") }, singleLine = true)
                Text("路径属于执行设备。此端不运行 Harness。", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = { TextButton(onClick = { onDismiss(); model.createConversation(cwd) }, enabled = state.runtimeId != null && state.pending == null && !state.busy) { Text("创建") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
    )
}

@Composable
private fun ChatScreen(state: RemoteUiState, model: RemoteViewModel) {
    val snapshot = state.snapshot ?: return
    var draft by remember(snapshot.conversation.id) { mutableStateOf("") }
    val listState = rememberLazyListState()
    val running = snapshot.runState == BindingRunState.RUNNING || snapshot.runState == BindingRunState.STOPPING || snapshot.runState == BindingRunState.ELSEWHERE
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            if (running) CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
            Text(when (snapshot.runState) {
                BindingRunState.IDLE -> "就绪"
                BindingRunState.RUNNING -> "运行中"
                BindingRunState.STOPPING -> "正在停止"
                BindingRunState.FAILED -> "执行失败"
                BindingRunState.ELSEWHERE -> "运行中"
            }, Modifier.weight(1f).padding(8.dp), style = MaterialTheme.typography.labelMedium)
            if (snapshot.actions.canCancel) TextButton(onClick = model::cancel, enabled = !state.busy && state.pending == null) { Text("停止") }
        }
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), state = listState, contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(snapshot.messages, key = { it.id }) { original ->
                val expanded = state.expandedMessages[original.id]?.takeIf { it.stamp == original.stamp } ?: original
                MessageRow(expanded) { model.loadMessage(original.id) }
            }
            items(snapshot.pendingInteractions, key = { it.id }) { interaction ->
                InteractionCard(interaction, !state.busy && state.pending == null) { reply -> model.reply(interaction, reply) }
            }
            if (snapshot.messages.isEmpty()) item { Text("发送第一条消息开始。", color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        HorizontalDivider()
        Column(Modifier.fillMaxWidth().imePadding().padding(horizontal = 12.dp, vertical = 8.dp)) {
            RuntimeAndModel(state, model)
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(draft, { draft = it }, Modifier.weight(1f), label = { Text("消息") }, maxLines = 5, enabled = snapshot.actions.canSend && state.pending == null)
                Button(onClick = { val sent = draft; model.send(sent) { if (draft == sent) draft = "" } }, enabled = draft.isNotBlank() && snapshot.actions.canSend && !state.busy && state.pending == null) { Text("发送") }
            }
            if (!snapshot.actions.canSend) Text("此会话当前只读。", style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
private fun MessageRow(message: BindingMessage, load: () -> Unit) {
    val alignment = when (message.role) {
        BindingMessageRole.USER -> Alignment.End
        BindingMessageRole.ASSISTANT -> Alignment.Start
        BindingMessageRole.TOOL, BindingMessageRole.SYSTEM -> Alignment.CenterHorizontally
    }
    Column(Modifier.fillMaxWidth(), horizontalAlignment = alignment) {
        Surface(
            color = if (message.role == BindingMessageRole.USER) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerLow,
            shape = MaterialTheme.shapes.medium,
            modifier = Modifier.widthIn(max = 600.dp),
        ) {
            SelectionContainer {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    message.blocks.forEach { block ->
                        when (block) {
                            is BindingMessageBlock.Text -> Text(block.text)
                            is BindingMessageBlock.Notice -> Text(block.text, style = MaterialTheme.typography.bodySmall)
                            is BindingMessageBlock.Reasoning -> ExpandablePayload(block.summary.ifEmpty { "思考" }, block.text, load)
                            is BindingMessageBlock.Tool -> {
                                Text(block.title, style = MaterialTheme.typography.labelLarge)
                                block.output?.let { ExpandablePayload("查看工具输出", it, load) }
                            }
                            is BindingMessageBlock.Image -> Text("图片附件", style = MaterialTheme.typography.labelMedium)
                            is BindingMessageBlock.File -> Text("附件：${block.path.substringAfterLast('/')}", style = MaterialTheme.typography.labelMedium)
                            is BindingMessageBlock.AgentSender -> Text("来自 ${block.runtimeName} 会话「${block.title}」", style = MaterialTheme.typography.labelMedium)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ExpandablePayload(title: String, payload: BindingMessagePayload, load: () -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    TextButton(onClick = { expanded = !expanded; if (expanded && payload is BindingMessagePayload.Deferred) load() }) { Text(if (expanded) "收起 · $title" else title) }
    if (expanded) when (payload) {
        is BindingMessagePayload.Inline -> Text(payload.text, style = MaterialTheme.typography.bodySmall)
        is BindingMessagePayload.Deferred -> Text("正在读取 ${payload.bytes} 字节…", style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
private fun InteractionCard(interaction: BindingRuntimeInteraction, enabled: Boolean, reply: (BindingRuntimeInteractionReply) -> Unit) {
    OutlinedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            when (val kind = interaction.kind) {
                is BindingInteractionKind.Approval -> {
                    Text(kind.title, style = MaterialTheme.typography.titleMedium)
                    SelectionContainer { Text(kind.detail) }
                    kind.options.forEach { option -> Button(onClick = { reply(BindingRuntimeInteractionReply.Decision(option.id)) }, enabled = enabled) { Text(option.label) } }
                }
                is BindingInteractionKind.UserInput -> {
                    Text("需要回答", style = MaterialTheme.typography.titleMedium)
                    val answers = remember(interaction.id) { mutableStateMapOf<String, List<String>>() }
                    kind.questions.forEach { question ->
                        Text(question.text)
                        if (question.options.isEmpty()) {
                            OutlinedTextField(answers[question.id]?.firstOrNull().orEmpty(), { answers[question.id] = listOf(it) }, Modifier.fillMaxWidth(), enabled = enabled, visualTransformation = if (question.secret) PasswordVisualTransformation() else VisualTransformation.None)
                        } else {
                            question.options.forEach { option ->
                                Row(Modifier.fillMaxWidth().clickable(enabled = enabled) { answers[question.id] = listOf(option.id) }, verticalAlignment = Alignment.CenterVertically) {
                                    RadioButton(answers[question.id]?.contains(option.id) == true, onClick = { answers[question.id] = listOf(option.id) }, enabled = enabled)
                                    Text(option.label)
                                }
                            }
                        }
                    }
                    Button(onClick = { reply(BindingRuntimeInteractionReply.Answers(kind.questions.map { BindingInteractionAnswer(it.id, answers[it.id].orEmpty()) })) }, enabled = enabled && kind.questions.all { !answers[it.id].isNullOrEmpty() }) { Text("提交回答") }
                }
            }
            TextButton(onClick = { reply(BindingRuntimeInteractionReply.Cancel) }, enabled = enabled) { Text("取消交互") }
        }
    }
}
