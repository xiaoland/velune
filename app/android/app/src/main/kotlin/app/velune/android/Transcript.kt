package app.velune.android

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.CallSplit
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.Lightbulb
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.application.MessagePayload
import app.velune.bindings.conversation.MessageRole
import app.velune.bindings.conversation.ToolState
import app.velune.bindings.conversation.TurnOutcome
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The conversation, newest at the bottom. The list lays out in reverse so a
 * growing reply stays anchored to the bottom while the user is there, and
 * stays put once they scroll up to read.
 */
@Composable
fun Transcript(
    state: ConversationState,
    list: LazyListState,
    load: (String) -> Unit,
    onEdit: (DeliveredMessage) -> Unit = {},
    onFork: (DeliveredMessage) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val spacing = LocalAppearance.current.spacingScale
    // Explicit user toggles outlive rows scrolled out of the lazy list.
    val expanded = remember { mutableStateMapOf<String, Boolean>() }
    val awayFromEnd by remember { derivedStateOf { list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 240 } }
    val working = state.running && (state.entries.lastOrNull() as? TranscriptEntry.Work)?.running != true
    // Sending jumps back to the end, where the message appears.
    LaunchedEffect(state.sending) { if (state.sending != null) list.scrollToItem(0) }
    BoxWithConstraints(modifier) {
        val bubbleWidth = maxWidth * 0.85f
        LazyColumn(
            state = list, reverseLayout = true, modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(horizontal = 16.dp * spacing, vertical = 12.dp * spacing),
            verticalArrangement = Arrangement.spacedBy(16.dp * spacing, Alignment.Bottom),
        ) {
            if (working) item(key = "working") { Working() }
            state.sending?.let { text -> item(key = "sending") { UserMessage(listOf(DeliveredBlock.Text(text)), null, bubbleWidth, pending = true) } }
            items(state.entries.asReversed(), key = { it.key }) { entry ->
                when (entry) {
                    is TranscriptEntry.Message -> MessageView(
                        entry.message, bubbleWidth, expanded, load,
                        editable = entry.message.id in state.editable,
                        forkable = entry.canFork && state.canFork,
                        onEdit = { onEdit(entry.message) },
                        onFork = { onFork(entry.message) },
                    )
                    is TranscriptEntry.Work -> WorkGroup(entry, expanded, load)
                }
            }
        }
        if (awayFromEnd) {
            SmallFloatingActionButton(
                onClick = { scope.launch { list.scrollToItem(0) } },
                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                contentColor = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp * spacing),
            ) { Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "回到最新") }
        }
    }
}

/** The user's messages on the right, the model's on the left, the system's and the Harness's in the middle. */
@Composable
private fun MessageView(
    message: DeliveredMessage,
    bubbleWidth: Dp,
    expanded: MutableMap<String, Boolean>,
    load: (String) -> Unit,
    editable: Boolean,
    forkable: Boolean,
    onEdit: () -> Unit,
    onFork: () -> Unit,
) {
    val spacing = LocalAppearance.current.spacingScale
    when (message.role) {
        MessageRole.USER, MessageRole.CLIENT -> UserMessage(message.blocks, message.timestampUnixMs, bubbleWidth, onEdit = onEdit.takeIf { editable })
        MessageRole.ASSISTANT -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp * spacing)) {
            message.blocks.forEachIndexed { index, block -> WorkBlock(message.id, index, block, expanded, load, reply = true) }
            val copyable = remember(message.blocks) { message.copyableText() }
            if (message.completed && (copyable.isNotEmpty() || message.timestampUnixMs != null || forkable)) {
                MessageMetaRow(message.timestampUnixMs, copyable.takeIf { it.isNotEmpty() }, Alignment.Start, onFork = onFork.takeIf { forkable })
            }
        }
        MessageRole.TOOL, MessageRole.SYSTEM -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp * spacing)) {
            message.blocks.forEachIndexed { index, block -> WorkBlock(message.id, index, block, expanded, load, reply = false) }
        }
    }
}

@Composable
private fun UserMessage(
    blocks: List<DeliveredBlock>,
    timestampUnixMs: Long?,
    maxWidth: Dp,
    pending: Boolean = false,
    onEdit: (() -> Unit)? = null,
) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    val copyable = remember(blocks) {
        blocks.filterIsInstance<DeliveredBlock.Text>().mapNotNull { it.text.takeIf(String::isNotBlank) }.joinToString("\n\n")
    }
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End) {
        Surface(
            color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            shape = RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp, bottomStart = 18.dp, bottomEnd = 4.dp),
            modifier = Modifier.widthIn(max = maxWidth).alpha(if (pending) 0.6f else 1f),
        ) {
            Column(Modifier.padding(horizontal = 14.dp * spacing, vertical = 10.dp * spacing), verticalArrangement = Arrangement.spacedBy(6.dp * spacing)) {
                blocks.forEach { block ->
                    when (block) {
                        is DeliveredBlock.Text -> SelectionContainer { Text(block.text, style = BaseTypography.bodyLarge.scaled(appearance.messageScale)) }
                        is DeliveredBlock.Image -> Attachment(Icons.Outlined.Image, "图片")
                        is DeliveredBlock.File -> Attachment(Icons.AutoMirrored.Outlined.InsertDriveFile, lastPathComponent(block.path))
                        else -> {}
                    }
                }
            }
        }
        if (pending) {
            Text("发送中…", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 4.dp * spacing, end = 4.dp * spacing))
        } else if (copyable.isNotEmpty() || timestampUnixMs != null || onEdit != null) {
            MessageMetaRow(timestampUnixMs, copyable.takeIf { it.isNotEmpty() }, Alignment.End, onEdit = onEdit)
        }
    }
}

/** Quiet timestamp and one-tap actions (copy, edit, fork) under user messages and completed replies. */
@Composable
private fun MessageMetaRow(
    timestampUnixMs: Long?,
    copyText: String?,
    alignment: Alignment.Horizontal,
    onEdit: (() -> Unit)? = null,
    onFork: (() -> Unit)? = null,
) {
    val copy = rememberCopy()
    val spacing = LocalAppearance.current.spacingScale
    val time = remember(timestampUnixMs) { timestampUnixMs?.let(::formatMessageTime) }
    Row(
        Modifier.fillMaxWidth().padding(top = 2.dp * spacing),
        horizontalArrangement = if (alignment == Alignment.End) Arrangement.End else Arrangement.Start,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (alignment == Alignment.End) {
            if (onEdit != null) IconButton(onClick = onEdit, modifier = Modifier.size(32.dp)) {
                Icon(Icons.Outlined.Edit, contentDescription = "编辑消息", modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (copyText != null) CopyButton { copy(copyText) }
            if (time != null) Text(time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            if (time != null) Text(time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (copyText != null) {
                if (time != null) Spacer(Modifier.width(4.dp * spacing))
                CopyButton { copy(copyText) }
            }
            if (onFork != null) IconButton(onClick = onFork, modifier = Modifier.size(32.dp)) {
                Icon(Icons.AutoMirrored.Outlined.CallSplit, contentDescription = "分叉会话", modifier = Modifier.size(15.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun CopyButton(onClick: () -> Unit) {
    var copied by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    IconButton(
        onClick = {
            onClick()
            copied = true
            scope.launch { delay(1_500); copied = false }
        },
        modifier = Modifier.size(32.dp),
    ) {
        Icon(
            if (copied) Icons.Filled.Check else Icons.Outlined.ContentCopy,
            contentDescription = if (copied) "已复制" else "复制",
            modifier = Modifier.size(15.dp),
            tint = if (copied) Tone.Positive.color() else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * A turn's work: open while running or when it failed/stopped, folded into one
 * quiet line once finished cleanly. Unfolded steps hang off a left rule.
 * A recovered tool failure stays inside the steps rather than alarming a finished turn.
 */
@Composable
private fun WorkGroup(work: TranscriptEntry.Work, expanded: MutableMap<String, Boolean>, load: (String) -> Unit) {
    val spacing = LocalAppearance.current.spacingScale
    val open = expanded[work.key] ?: work.defaultExpanded
    val latest = remember(work) { work.members.flatMap { it.blocks }.lastOrNull { it is DeliveredBlock.Tool } as? DeliveredBlock.Tool }
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier.clip(RoundedCornerShape(8.dp)).clickable { expanded[work.key] = !open }.heightIn(min = 36.dp)
                .padding(vertical = 4.dp * spacing, horizontal = 2.dp * spacing),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (work.running) CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
            else Icon(if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = if (open) "收起工作过程" else "展开工作过程", modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            val parts = listOfNotNull(
                if (work.running) "正在工作" else "工作过程",
                if (work.running && !open) latest?.title else work.toolCount.takeIf { it > 0 }?.let { "$it 个工具" },
                work.durationMs?.takeIf { !work.running }?.let(::duration),
            )
            Text(
                parts.joinToString(" · "), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp * spacing).weight(1f, fill = false),
            )
            if (work.outcome == TurnOutcome.FAILED) Text(" · 失败", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.error, maxLines = 1)
            else if (work.outcome == TurnOutcome.CANCELLED) Text(" · 已停止", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
        }
        if (open) Row(Modifier.height(IntrinsicSize.Min).padding(top = 4.dp * spacing)) {
            Box(Modifier.padding(start = 8.dp * spacing, end = 12.dp * spacing).width(1.5.dp).fillMaxHeight().background(MaterialTheme.colorScheme.outlineVariant))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp * spacing)) {
                work.members.forEach { message ->
                    message.blocks.forEachIndexed { index, block -> WorkBlock(message.id, index, block, expanded, load, reply = false) }
                }
            }
        }
    }
}

@Composable
private fun WorkBlock(messageId: String, index: Int, block: DeliveredBlock, expanded: MutableMap<String, Boolean>, load: (String) -> Unit, reply: Boolean) {
    val key = "$messageId/$index"
    val appearance = LocalAppearance.current
    when (block) {
        is DeliveredBlock.Text -> SelectionContainer {
            MessageMarkdown(block.text, baseStyle = if (reply) BaseTypography.bodyLarge else BaseTypography.bodyMedium)
        }
        is DeliveredBlock.Reasoning -> if (block.summary.isNotBlank()) Reasoning(block, expanded[key] == true, toggle = {
            expanded[key] = expanded[key] != true
            if (block.text is MessagePayload.Deferred) load(messageId)
        })
        is DeliveredBlock.Tool -> ToolStep(block, expanded[key] == true, toggle = {
            expanded[key] = expanded[key] != true
            if (block.output is MessagePayload.Deferred) load(messageId)
        })
        is DeliveredBlock.Notice -> Text(
            block.text, style = BaseTypography.bodySmall.scaled(appearance.messageScale), color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp * appearance.spacingScale),
        )
        is DeliveredBlock.RuntimeFailure -> Text(block.detail, style = BaseTypography.bodySmall.scaled(appearance.messageScale), color = MaterialTheme.colorScheme.error)
        is DeliveredBlock.ContextCompaction -> ContextCompaction(block.text, expanded[key] == true) {
            expanded[key] = expanded[key] != true
        }
        is DeliveredBlock.Image -> Attachment(Icons.Outlined.Image, "图片")
        is DeliveredBlock.File -> Attachment(Icons.AutoMirrored.Outlined.InsertDriveFile, lastPathComponent(block.path))
        is DeliveredBlock.AgentSender -> Text(
            "来自 ${block.runtimeName} · ${block.title}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Reasoning(block: DeliveredBlock.Reasoning, open: Boolean, toggle: () -> Unit) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable(onClick = toggle).heightIn(min = 36.dp).padding(vertical = 4.dp * spacing),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Outlined.Lightbulb, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                block.summary, style = BaseTypography.bodySmall.scaled(appearance.messageScale), color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = if (open) 4 else 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 8.dp * spacing).weight(1f),
            )
            Icon(
                if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null,
                modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline,
            )
        }
        if (open) Payload(block.text, italic = true)
    }
}

/** The runtime supplies a status line followed by the native summary, if any. */
private fun contextCompactionParts(text: String): Pair<String, String?> {
    val newline = text.indexOf('\n')
    if (newline < 0) return text to null
    val detail = text.substring(newline + 1).trimStart('\r', '\n')
    if (detail.isBlank()) return text to null
    return text.substring(0, newline).trimEnd('\r') to detail
}

@Composable
private fun ContextCompaction(text: String, open: Boolean, toggle: () -> Unit) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    val style = BaseTypography.bodySmall.scaled(appearance.messageScale)
    val (status, detail) = remember(text) { contextCompactionParts(text) }
    if (detail == null) {
        Text(
            text, style = style, color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp * spacing),
        )
    } else Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable(onClick = toggle).heightIn(min = 40.dp).padding(vertical = 4.dp * spacing),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = if (open) "收起详情" else "展开详情",
                modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(status, style = style, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 6.dp * spacing).weight(1f))
        }
        if (open) SelectionContainer { MessageMarkdown(detail, baseStyle = BaseTypography.bodySmall) }
    }
}

/** One tool call on the work rail: compact and scannable; output unfolds below on demand. */
@Composable
private fun ToolStep(block: DeliveredBlock.Tool, open: Boolean, toggle: () -> Unit) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable(enabled = block.output != null, onClick = toggle).heightIn(min = 38.dp)
                .padding(vertical = 6.dp * spacing, horizontal = 2.dp * spacing),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            when (block.state) {
                ToolState.PENDING -> Icon(Icons.Outlined.Schedule, contentDescription = "等待", modifier = Modifier.size(16.dp), tint = Tone.Attention.color())
                ToolState.RUNNING -> CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
                ToolState.COMPLETED -> Icon(Icons.Outlined.CheckCircle, contentDescription = "完成", modifier = Modifier.size(16.dp), tint = Tone.Positive.color())
                ToolState.FAILED -> Icon(Icons.Outlined.ErrorOutline, contentDescription = "失败", modifier = Modifier.size(16.dp), tint = Tone.Negative.color())
            }
            Text(
                block.title, style = BaseTypography.bodySmall.scaled(appearance.codeScale), fontFamily = FontFamily.Monospace,
                color = if (block.state == ToolState.FAILED) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                maxLines = if (open) 6 else 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 8.dp * spacing).weight(1f),
            )
            if (block.output != null) Icon(
                if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null,
                modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline,
            )
        }
        if (open) block.output?.let { Box(Modifier.padding(vertical = 4.dp * spacing)) { Payload(it, italic = false) } }
    }
}

@Composable
private fun Payload(payload: MessagePayload, italic: Boolean) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    when (payload) {
        is MessagePayload.Inline -> if (italic) SelectionContainer {
            Text(
                payload.text, style = BaseTypography.bodySmall.scaled(appearance.messageScale), fontStyle = FontStyle.Italic,
                color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp * spacing, top = 2.dp * spacing, bottom = 4.dp * spacing),
            )
        } else Output(payload.text)
        is MessagePayload.Deferred -> Row(Modifier.padding(8.dp * spacing), verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
            Text("正在读取 ${bytes(payload.bytes)}…", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp * spacing))
        }
    }
}

/** Tool output: monospace, unwrapped, at most a screenful tall. */
@Composable
private fun Output(text: String) {
    val copy = rememberCopy()
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(8.dp), modifier = Modifier.fillMaxWidth()) {
        Box {
            SelectionContainer {
                Text(
                    text.trimEnd(), style = BaseTypography.bodySmall.scaled(appearance.codeScale), fontFamily = FontFamily.Monospace, softWrap = false,
                    modifier = Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState())
                        .padding(10.dp * spacing).padding(end = 28.dp),
                )
            }
            IconButton(onClick = { copy(text) }, modifier = Modifier.align(Alignment.TopEnd).size(36.dp)) {
                Icon(Icons.Outlined.ContentCopy, contentDescription = "复制输出", modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun Attachment(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String) {
    val spacing = LocalAppearance.current.spacingScale
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHighest, shape = RoundedCornerShape(8.dp)) {
        Row(Modifier.padding(horizontal = 10.dp * spacing, vertical = 6.dp * spacing), verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(16.dp))
            Text(label, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp * spacing))
        }
    }
}

@Composable
private fun Working() {
    val spacing = LocalAppearance.current.spacingScale
    Row(verticalAlignment = Alignment.CenterVertically) {
        CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
        Text("正在运行…", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp * spacing))
    }
}

private fun bytes(count: ULong): String = when {
    count < 1_024uL -> "$count B"
    count < 1_048_576uL -> "${count / 1_024uL} KB"
    else -> "%.1f MB".format(count.toDouble() / 1_048_576.0)
}
