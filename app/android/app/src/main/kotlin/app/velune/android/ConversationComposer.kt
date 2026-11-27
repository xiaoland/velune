package app.velune.android

import app.velune.bindings.conversation.ControlHandoff
import app.velune.bindings.conversation.InputAccess
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.InputChip
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.application.RuntimeInstance
import app.velune.bindings.conversation.QueuedMessage
import app.velune.bindings.conversation.RunState

/** The input: text on top, the next turn's model/permission/effort and send/stop below, in one rounded container. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun ConversationComposer(
    state: ConversationState,
    actions: ConversationActions,
    runtimes: List<RuntimeInstance>,
    runtimeLabels: Map<String, RuntimeLabel>,
) {
    val text = state.draft
    val spacing = LocalAppearance.current.spacingScale
    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    val insets = if (imeVisible) Modifier.padding(top = 4.dp) else {
        Modifier.navigationBarsPadding().padding(start = 12.dp, end = 12.dp, top = 4.dp, bottom = 8.dp)
    }
    val shape = if (imeVisible) {
        RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp, bottomStart = 0.dp, bottomEnd = 0.dp)
    } else RoundedCornerShape(24.dp)
    val submitColors = IconButtonDefaults.filledIconButtonColors(
        containerColor = MaterialTheme.colorScheme.primary,
        contentColor = MaterialTheme.colorScheme.onPrimary,
        disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
        disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    var choosingModel by remember { mutableStateOf(false) }
    var choosingConfig by remember { mutableStateOf(false) }
    var showQueue by remember { mutableStateOf(false) }
    val hasInput = text.isNotBlank() || state.attachments.isNotEmpty()
    val canSubmit = state.composerReady && state.composerFailure == null && !state.busy &&
        !state.uploadingAttachment && !state.switchingRuntime && hasInput
    val showRuntimeOptions = state.canSwitch && (runtimes.size >= 2 || runtimes.any { it.id != state.selectedRuntimeId })
    val hasExecutionConfig = showRuntimeOptions || state.permissionModes.isNotEmpty() || state.efforts.isNotEmpty()
    val canChooseModel = !state.busy && !state.switchingRuntime &&
        (state.composerReady || state.composerFailure != null)
    val defaultModelLabel = if (state.hasCurrent) {
        state.currentModelName?.let { "沿用当前（$it）" } ?: "沿用当前"
    } else "运行时默认"
    val shownModel = state.models.firstOrNull { it.selection == state.selection }?.name ?: state.currentModelName ?: "默认模型"
    val activePermission = state.permissionModes.firstOrNull { it.id == state.permissionMode } ?: state.currentPermissionMode
    val activeEffort = state.effort ?: state.currentEffort
    val selectedRuntime = runtimeLabels[state.selectedRuntimeId]?.name
    val configSummary = listOfNotNull(selectedRuntime.takeIf { showRuntimeOptions && !state.usesCurrentRuntime }, activePermission?.label, activeEffort)
        .joinToString(" · ").ifEmpty { "权限与推理" }
    Column(Modifier.fillMaxWidth().then(insets), verticalArrangement = Arrangement.spacedBy(6.dp * spacing)) {
        ComposerStatus(state, actions)
        if (state.queue.isNotEmpty()) QueuePreview(state.queue) { showQueue = true }
        if (state.attachments.isNotEmpty()) {
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                state.attachments.forEach { attachment ->
                    InputChip(
                        selected = false,
                        enabled = state.inputAccess != InputAccess.READ_ONLY,
                        onClick = { actions.removeAttachment(attachment.id) },
                        label = { Text(attachment.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        trailingIcon = { Icon(Icons.Filled.Close, contentDescription = "移除附件", modifier = Modifier.size(16.dp)) },
                    )
                }
            }
        }
        Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = shape, modifier = Modifier.fillMaxWidth()) {
            Column {
                if (imeVisible) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                val bodyStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface)
                BasicTextField(
                    value = text,
                    onValueChange = actions.editDraft,
                    readOnly = state.inputAccess == InputAccess.READ_ONLY,
                    textStyle = bodyStyle,
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    maxLines = 6,
                    modifier = Modifier.fillMaxWidth(),
                    decorationBox = { field ->
                        Box(
                            Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                .padding(start = 16.dp, end = 16.dp, top = 12.dp * spacing, bottom = 6.dp * spacing),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            if (text.isEmpty()) Text("给 Agent 发消息", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            field()
                        }
                    },
                )
                Row(
                    Modifier.fillMaxWidth().padding(start = 10.dp, end = 8.dp, bottom = 8.dp * spacing),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    ComposerAttachmentButton(
                        enabled = state.inputAccess != InputAccess.READ_ONLY && !state.busy,
                        uploading = state.uploadingAttachment,
                        addAttachments = actions.addAttachments,
                    )
                    Row(
                        Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        if (state.models.isNotEmpty()) {
                            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                                Row(
                                    Modifier.clickable(enabled = canChooseModel) { choosingModel = true }.padding(start = 12.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
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
                                    Modifier.clickable(enabled = !state.busy && (state.composerReady || showRuntimeOptions)) { choosingConfig = true }.padding(start = 10.dp, end = 10.dp, top = 6.dp, bottom = 6.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Icon(Icons.Outlined.Tune, contentDescription = null, modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Text(configSummary, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, modifier = Modifier.padding(start = 5.dp))
                                }
                            }
                        }
                    }
                    if (state.running) {
                        if (hasInput && state.canSteer && state.usesCurrentRuntime) {
                            TextButton(enabled = canSubmit, onClick = { actions.steer(text.trim()) }) { Text("介入") }
                        }
                        if (hasInput && state.canQueue) {
                            TextButton(enabled = canSubmit, onClick = { actions.queue(text.trim()) }) { Text("排队") }
                        }
                        if (state.canCancel) {
                            FilledIconButton(enabled = !state.busy, onClick = actions.stop, modifier = Modifier.size(40.dp), shape = CircleShape, colors = submitColors) {
                                Icon(Icons.Filled.Stop, contentDescription = "停止")
                            }
                        }
                    } else {
                        FilledIconButton(
                            enabled = canSubmit && state.canSend, onClick = { actions.send(text.trim()) {} }, modifier = Modifier.size(40.dp),
                            shape = CircleShape, colors = submitColors,
                        ) {
                            if (state.busy && state.sending != null) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            else Icon(Icons.Filled.ArrowUpward, contentDescription = "发送")
                        }
                    }
                }
            }
        }
    }
    if (showQueue) ConversationQueueSheet(state.queue) { showQueue = false }
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
            if (showRuntimeOptions) {
                Text(
                    "运行时",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 24.dp, top = 8.dp * spacing, bottom = 4.dp * spacing),
                )
                FlowRow(Modifier.padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    runtimes.forEach { runtime ->
                        FilterChip(
                            selected = runtime.id == state.selectedRuntimeId,
                            enabled = !state.switchingRuntime && !state.busy,
                            onClick = { actions.selectRuntime(runtime.id) },
                            label = { Text(runtimeLabels[runtime.id]?.name ?: runtime.name) },
                        )
                    }
                }
            }
            if (state.switchingRuntime || state.composerFailure != null) {
                Text(
                    if (state.switchingRuntime) "正在切换运行时…" else state.composerFailure.orEmpty(),
                    style = MaterialTheme.typography.bodySmall,
                    color = if (state.switchingRuntime) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp * spacing),
                )
            }
            if (state.efforts.isNotEmpty()) {
                val defaultEffort = if (state.hasCurrent) {
                    state.currentEffort?.let { "沿用当前（$it）" } ?: "沿用当前"
                } else "运行时默认"
                Text("推理强度", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 8.dp, bottom = 8.dp))
                FlowRow(Modifier.padding(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(selected = state.effort == null, enabled = canChooseModel, onClick = { actions.selectEffort(null) }, label = { Text(defaultEffort) })
                    state.efforts.forEach { level ->
                        FilterChip(selected = state.effort == level, enabled = canChooseModel, onClick = { actions.selectEffort(level) }, label = { Text(level) })
                    }
                }
            }
            if (state.permissionModes.isNotEmpty()) {
                val defaultPermission = if (state.hasCurrent) {
                    state.currentPermissionMode?.label?.let { "沿用当前（$it）" } ?: "沿用当前"
                } else "运行时默认"
                Text("权限模式", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 4.dp))
                PermissionOption(defaultPermission, null, state.permissionMode == null, canChooseModel) {
                    actions.selectPermissionMode(null)
                    choosingConfig = false
                }
                state.permissionModes.forEach { mode ->
                    PermissionOption(mode.label, mode.detail.takeIf { it.isNotBlank() }, state.permissionMode == mode.id, canChooseModel) {
                        actions.selectPermissionMode(mode.id)
                        choosingConfig = false
                    }
                }
            }
        }
    }
}

@Composable
private fun PermissionOption(title: String, detail: String?, selected: Boolean, enabled: Boolean, choose: () -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = detail?.let { text -> { Text(text) } },
        trailingContent = { if (selected) Icon(Icons.Filled.Check, contentDescription = "当前选择", tint = MaterialTheme.colorScheme.primary) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(enabled = enabled, onClick = choose).padding(horizontal = 8.dp),
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
        Text(
            if (state.switchingRuntime) "正在切换运行时…" else "正在恢复输入…",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 8.dp),
        )
    }
    state.composerFailure?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 8.dp))
    }
    state.submittingFollowUp?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 8.dp))
    }
    val runNotice = if (state.isCompacting) Tone.Active to "正在压缩上下文…" else when (state.runState) {
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
    if (!state.running && state.canResume && state.usesCurrentRuntime) {
        TextButton(
            onClick = actions.resumeTurn,
            enabled = state.composerReady && !state.busy && !state.switchingRuntime,
        ) { Text("继续执行") }
    }
}

@Composable
private fun QueuePreview(queue: List<QueuedMessage>, onClick: () -> Unit) {
    val spacing = LocalAppearance.current.spacingScale
    val first = queue.first()
    Surface(onClick = onClick, shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceContainerLow) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 40.dp).padding(horizontal = 10.dp, vertical = 4.dp * spacing),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("排队中（${queue.size}）", style = MaterialTheme.typography.labelMedium, maxLines = 1)
            Text(
                first.text.trim().ifEmpty { "${first.attachments.size} 个附件" },
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
            )
            Icon(Icons.Filled.ExpandMore, contentDescription = "查看排队消息", modifier = Modifier.size(18.dp))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConversationQueueSheet(queue: List<QueuedMessage>, onDismiss: () -> Unit) {
    val spacing = LocalAppearance.current.spacingScale
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Text("排队消息（${queue.size}）", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp * spacing))
        LazyColumn(Modifier.navigationBarsPadding().padding(bottom = 16.dp * spacing)) {
            if (queue.isEmpty()) item { ListItem(headlineContent = { Text("没有排队消息") }) }
            items(queue, key = { it.id }) { message ->
                val files = message.attachments.joinToString(" · ", transform = ::lastPathComponent)
                ListItem(
                    headlineContent = { Text(message.text.ifBlank { files }) },
                    supportingContent = if (message.text.isNotBlank() && files.isNotBlank()) ({ Text(files) }) else null,
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                )
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
