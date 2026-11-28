package app.velune.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.conversation.BackgroundTaskInfo
import app.velune.bindings.conversation.BackgroundTaskOutput
import app.velune.bindings.conversation.BackgroundTaskOutputStatus
import app.velune.bindings.conversation.BackgroundTaskStatus
import kotlinx.coroutines.delay

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun BackgroundTasksSheet(
    state: ConversationState,
    onDismiss: () -> Unit,
    onRefresh: () -> Unit,
    onLoadOutput: (String) -> Unit,
    onStop: (String) -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    val listed = state.backgroundTasks
    val live = listed?.live == true
    val tasks = listed?.tasks.orEmpty()
    val ordered = remember(tasks, live) { tasks.sortedByDescending { live && it.status.isActive } }
    val issue = listed?.issue
    var expandedId by remember(state.conversation?.id) { mutableStateOf<String?>(null) }
    val expanded = tasks.firstOrNull { it.id == expandedId }
    val outputStatus = expanded?.let { state.backgroundTaskOutputs[it.id]?.status }
    val canReadExpanded = live && expanded?.canReadOutput == true
    val outputFailed = expanded?.id in state.backgroundTaskOutputFailures
    val latestLoadOutput by rememberUpdatedState(onLoadOutput)

    LaunchedEffect(expanded?.id, canReadExpanded, outputStatus, outputFailed) {
        if (expanded == null || !canReadExpanded || outputFailed) return@LaunchedEffect
        when (outputStatus) {
            null -> latestLoadOutput(expanded.id)
            BackgroundTaskOutputStatus.CHECKING -> {
                while (true) {
                    delay(1_000)
                    latestLoadOutput(expanded.id)
                }
            }
            BackgroundTaskOutputStatus.READY, BackgroundTaskOutputStatus.FAILED -> Unit
        }
    }

    ModalBottomSheet(
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        onDismissRequest = onDismiss,
    ) {
        Row(
            Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("后台任务", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            IconButton(onClick = onRefresh, enabled = !state.loadingBackgroundTasks) {
                if (state.loadingBackgroundTasks) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                } else {
                    Icon(Icons.Filled.Refresh, contentDescription = "刷新后台任务")
                }
            }
        }
        state.backgroundTaskFailure?.let { Banner(it) }
        LazyColumn(Modifier.navigationBarsPadding().padding(bottom = 16.dp * spacing)) {
            if (issue != null) {
                item { BackgroundTaskNote(issue, Modifier.padding(horizontal = 24.dp, vertical = 8.dp * spacing)) }
            }
            if (ordered.isEmpty() && issue == null && state.backgroundTaskFailure == null) {
                item {
                    BackgroundTaskNote(
                        if (state.loadingBackgroundTasks) "正在读取后台任务…" else "没有后台任务",
                        Modifier.padding(horizontal = 24.dp, vertical = 12.dp * spacing),
                    )
                }
            }
            items(ordered, key = { it.id }) { task ->
                val isExpanded = task.id == expandedId
                val status = if (!live && task.status.isActive) BackgroundTaskStatus.UNKNOWN else task.status
                BackgroundTaskRow(
                    task = task,
                    status = status,
                    expanded = isExpanded,
                    stopping = task.id in state.stoppingBackgroundTaskIds,
                    onToggle = {
                        expandedId = if (isExpanded) null else task.id
                        if (!isExpanded && live && task.canReadOutput) onLoadOutput(task.id)
                    },
                    onStop = { onStop(task.id) },
                )
                if (isExpanded) {
                    BackgroundTaskOutputPanel(
                        output = state.backgroundTaskOutputs[task.id],
                        canRead = live && task.canReadOutput,
                        loading = task.id in state.loadingBackgroundTaskOutputs,
                        failure = state.backgroundTaskOutputFailures[task.id],
                        onRead = { onLoadOutput(task.id) },
                    )
                }
                HorizontalDivider(Modifier.padding(horizontal = 16.dp))
            }
        }
    }
}

private val BackgroundTaskStatus.isActive: Boolean
    get() = this == BackgroundTaskStatus.RUNNING || this == BackgroundTaskStatus.STOPPING

@Composable
private fun BackgroundTaskRow(
    task: BackgroundTaskInfo,
    status: BackgroundTaskStatus,
    expanded: Boolean,
    stopping: Boolean,
    onToggle: () -> Unit,
    onStop: () -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    val (label, tone) = when (status) {
        BackgroundTaskStatus.RUNNING -> "运行中" to Tone.Active
        BackgroundTaskStatus.STOPPING -> "正在停止" to Tone.Attention
        BackgroundTaskStatus.COMPLETED -> "已完成" to Tone.Positive
        BackgroundTaskStatus.FAILED -> "失败" to Tone.Negative
        BackgroundTaskStatus.CANCELLED -> "已停止" to Tone.Neutral
        BackgroundTaskStatus.UNKNOWN -> "状态未知" to Tone.Neutral
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        ListItem(
            headlineContent = {
                Text(
                    task.title.ifBlank { task.kind.ifBlank { "后台任务" } },
                    maxLines = if (expanded) Int.MAX_VALUE else 2,
                    overflow = TextOverflow.Ellipsis,
                )
            },
            supportingContent = {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp * spacing), verticalAlignment = Alignment.CenterVertically) {
                    StatusDot(tone, size = 6.dp)
                    Text(label)
                }
            },
            trailingContent = { Icon(if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore, contentDescription = null) },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            modifier = Modifier.weight(1f).clickable(
                onClickLabel = if (expanded) "收起任务输出" else "查看任务输出",
                onClick = onToggle,
            ),
        )
        if (status == BackgroundTaskStatus.RUNNING && task.canStop) {
            TextButton(onClick = onStop, enabled = !stopping) {
                Text(if (stopping) "正在停止" else "停止")
            }
        }
    }
}

@Composable
private fun BackgroundTaskOutputPanel(
    output: BackgroundTaskOutput?,
    canRead: Boolean,
    loading: Boolean,
    failure: String?,
    onRead: () -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    Column(Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, bottom = 12.dp * spacing)) {
        when {
            loading || canRead && output?.status == BackgroundTaskOutputStatus.CHECKING && failure == null -> {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp * spacing), verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    BackgroundTaskNote("正在读取输出…")
                }
            }
            failure != null -> {
                BackgroundTaskNote(failure)
                if (canRead) TextButton(onClick = onRead) { Text("重试") }
            }
            output?.status == BackgroundTaskOutputStatus.READY -> BackgroundTaskOutputText(output)
            output?.status == BackgroundTaskOutputStatus.FAILED -> {
                BackgroundTaskNote(output.issue ?: "无法读取任务输出")
                if (canRead) TextButton(onClick = onRead) { Text("重试") }
            }
            canRead -> TextButton(onClick = onRead) { Text("读取输出") }
            else -> BackgroundTaskNote("当前没有可读取的输出")
        }
    }
}

@Composable
private fun BackgroundTaskOutputText(output: BackgroundTaskOutput) {
    if (output.text.isEmpty()) {
        BackgroundTaskNote("暂时没有输出")
        return
    }
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    val copy = rememberCopy()
    val bytes = remember(output.text) { output.text.toByteArray(Charsets.UTF_8).size }
    val sizeLabel = "$bytes 字节" + if (output.truncated) "（已截取）" else ""
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.small,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column {
            Row(
                Modifier.fillMaxWidth().padding(start = 12.dp * spacing),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                BackgroundTaskNote(sizeLabel, Modifier.weight(1f))
                IconButton(onClick = { copy(output.text) }) {
                    Icon(Icons.Outlined.ContentCopy, contentDescription = "复制输出", modifier = Modifier.size(18.dp))
                }
            }
            SelectionContainer {
                Text(
                    output.text,
                    modifier = Modifier.horizontalScroll(rememberScrollState())
                        .padding(start = 12.dp * spacing, end = 12.dp * spacing, bottom = 12.dp * spacing),
                    softWrap = false,
                    style = BaseTypography.bodySmall.scaled(appearance.codeScale),
                    fontFamily = FontFamily.Monospace,
                )
            }
        }
    }
}

@Composable
private fun BackgroundTaskNote(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
