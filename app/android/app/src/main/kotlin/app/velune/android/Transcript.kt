package app.velune.android

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ContentCopy
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
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingMessage
import app.velune.bindings.BindingMessageBlock
import app.velune.bindings.BindingMessagePayload
import app.velune.bindings.BindingMessageRole
import app.velune.bindings.BindingToolState
import kotlinx.coroutines.launch

/**
 * The conversation, newest at the bottom. The list lays out in reverse so a
 * growing reply stays anchored to the bottom while the user is there, and
 * stays put once they scroll up to read.
 */
@Composable
fun Transcript(state: ConversationState, load: (String) -> Unit, modifier: Modifier = Modifier) {
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    // Folded state outlives rows scrolled out of the lazy list.
    val expanded = remember { mutableStateMapOf<String, Boolean>() }
    val awayFromEnd by remember { derivedStateOf { list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 240 } }
    val working = state.running && (state.entries.lastOrNull() as? TranscriptEntry.Work)?.running != true
    // Sending jumps back to the end, where the message appears.
    LaunchedEffect(state.sending) { if (state.sending != null) list.animateScrollToItem(0) }
    BoxWithConstraints(modifier) {
        val bubbleWidth = maxWidth * 0.85f
        LazyColumn(
            state = list, reverseLayout = true, modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp, Alignment.Bottom),
        ) {
            if (working) item(key = "working") { Working() }
            state.sending?.let { text -> item(key = "sending") { UserBubble(listOf(BindingMessageBlock.Text(text)), bubbleWidth, pending = true) } }
            items(state.entries.asReversed(), key = { it.key }) { entry ->
                when (entry) {
                    is TranscriptEntry.Message -> MessageView(entry.message, bubbleWidth, expanded, load)
                    is TranscriptEntry.Work -> WorkGroup(entry, expanded, load)
                }
            }
        }
        AnimatedVisibility(
            visible = awayFromEnd, enter = fadeIn() + scaleIn(), exit = fadeOut() + scaleOut(),
            modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
        ) {
            SmallFloatingActionButton(
                onClick = { scope.launch { list.animateScrollToItem(0) } },
                containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
            ) { Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "回到最新") }
        }
    }
}

/** The user's messages on the right, the model's on the left, the system's and the Harness's in the middle. */
@Composable
private fun MessageView(message: BindingMessage, bubbleWidth: androidx.compose.ui.unit.Dp, expanded: MutableMap<String, Boolean>, load: (String) -> Unit) {
    when (message.role) {
        BindingMessageRole.USER -> UserBubble(message.blocks, bubbleWidth)
        BindingMessageRole.ASSISTANT, BindingMessageRole.TOOL, BindingMessageRole.SYSTEM -> Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            message.blocks.forEachIndexed { index, block -> WorkBlock(message.id, index, block, expanded, load, reply = message.role == BindingMessageRole.ASSISTANT) }
        }
    }
}

@Composable
private fun UserBubble(blocks: List<BindingMessageBlock>, maxWidth: androidx.compose.ui.unit.Dp, pending: Boolean = false) {
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End) {
        Surface(
            color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            shape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp, bottomStart = 20.dp, bottomEnd = 6.dp),
            modifier = Modifier.widthIn(max = maxWidth).alpha(if (pending) 0.6f else 1f),
        ) {
            Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                blocks.forEach { block ->
                    when (block) {
                        is BindingMessageBlock.Text -> SelectionContainer { Text(block.text, style = MaterialTheme.typography.bodyLarge) }
                        is BindingMessageBlock.Image -> Attachment(Icons.Outlined.Image, "图片")
                        is BindingMessageBlock.File -> Attachment(Icons.AutoMirrored.Outlined.InsertDriveFile, lastPathComponent(block.path))
                        else -> {}
                    }
                }
            }
        }
        if (pending) Text("发送中…", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 4.dp, end = 4.dp))
    }
}

/**
 * A turn's work in one line, like “Finished working” in Lody: what ran and
 * for how long. While it runs the line names the latest step; unfolded, the
 * steps hang off a rule on the left.
 */
@Composable
private fun WorkGroup(work: TranscriptEntry.Work, expanded: MutableMap<String, Boolean>, load: (String) -> Unit) {
    val open = expanded[work.key] == true
    val latest = remember(work) { work.members.flatMap { it.blocks }.lastOrNull { it is BindingMessageBlock.Tool } as? BindingMessageBlock.Tool }
    Column(Modifier.fillMaxWidth().animateContentSize()) {
        Row(
            Modifier.clip(RoundedCornerShape(8.dp)).clickable { expanded[work.key] = !open }.padding(vertical = 6.dp, horizontal = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (work.running) CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
            else Icon(if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null, modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            val parts = listOfNotNull(
                if (work.running) "正在工作" else "工作过程",
                if (work.running) latest?.title else work.toolCount.takeIf { it > 0 }?.let { "$it 个工具" },
                work.durationMs?.takeIf { !work.running }?.let(::duration),
            )
            Text(
                parts.joinToString(" · "), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp).weight(1f, fill = false),
            )
            if (work.failedTools > 0) Text(" · ${work.failedTools} 个失败", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.error, maxLines = 1)
        }
        if (open) Row(Modifier.height(IntrinsicSize.Min).padding(top = 4.dp)) {
            Box(Modifier.padding(start = 10.dp, end = 12.dp).width(1.5.dp).fillMaxHeight().background(MaterialTheme.colorScheme.outlineVariant))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                work.members.forEach { message ->
                    message.blocks.forEachIndexed { index, block -> WorkBlock(message.id, index, block, expanded, load, reply = false) }
                }
            }
        }
    }
}

@Composable
private fun WorkBlock(messageId: String, index: Int, block: BindingMessageBlock, expanded: MutableMap<String, Boolean>, load: (String) -> Unit, reply: Boolean) {
    val key = "$messageId/$index"
    when (block) {
        is BindingMessageBlock.Text -> SelectionContainer {
            Markdown(block.text, style = if (reply) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.bodyMedium)
        }
        is BindingMessageBlock.Reasoning -> Reasoning(block, expanded[key] == true, toggle = {
            expanded[key] = expanded[key] != true
            if (block.text is BindingMessagePayload.Deferred) load(messageId)
        })
        is BindingMessageBlock.Tool -> ToolCard(block, expanded[key] == true, toggle = {
            expanded[key] = expanded[key] != true
            if (block.output is BindingMessagePayload.Deferred) load(messageId)
        })
        is BindingMessageBlock.Notice -> Text(
            block.text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp),
        )
        is BindingMessageBlock.ContextCompaction -> Text(
            block.text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp),
        )
        is BindingMessageBlock.Image -> Attachment(Icons.Outlined.Image, "图片")
        is BindingMessageBlock.File -> Attachment(Icons.AutoMirrored.Outlined.InsertDriveFile, lastPathComponent(block.path))
        is BindingMessageBlock.AgentSender -> Text(
            "来自 ${block.runtimeName} · ${block.title}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Reasoning(block: BindingMessageBlock.Reasoning, open: Boolean, toggle: () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.clip(RoundedCornerShape(8.dp)).clickable(onClick = toggle).padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.Lightbulb, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                block.summary.ifBlank { "思考" }, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = if (open) 4 else 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp),
            )
        }
        if (open) Payload(block.text, italic = true)
    }
}

/** One tool call: its state and title; the output unfolds below, read on request when large. */
@Composable
private fun ToolCard(block: BindingMessageBlock.Tool, open: Boolean, toggle: () -> Unit) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(12.dp), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.animateContentSize()) {
            Row(
                Modifier.fillMaxWidth().clickable(enabled = block.output != null, onClick = toggle).heightIn(min = 44.dp).padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                when (block.state) {
                    BindingToolState.PENDING -> Icon(Icons.Outlined.Schedule, contentDescription = "等待", modifier = Modifier.size(18.dp), tint = Tone.Attention.color())
                    BindingToolState.RUNNING -> CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 1.5.dp)
                    BindingToolState.COMPLETED -> Icon(Icons.Outlined.CheckCircle, contentDescription = "完成", modifier = Modifier.size(18.dp), tint = Tone.Positive.color())
                    BindingToolState.FAILED -> Icon(Icons.Outlined.ErrorOutline, contentDescription = "失败", modifier = Modifier.size(18.dp), tint = Tone.Negative.color())
                }
                Text(
                    block.title, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace,
                    maxLines = if (open) 6 else 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 10.dp).weight(1f),
                )
                if (block.output != null) Icon(
                    if (open) Icons.Filled.KeyboardArrowDown else Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null,
                    modifier = Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (open) block.output?.let { Box(Modifier.padding(start = 12.dp, end = 12.dp, bottom = 12.dp)) { Payload(it, italic = false) } }
        }
    }
}

@Composable
private fun Payload(payload: BindingMessagePayload, italic: Boolean) {
    when (payload) {
        is BindingMessagePayload.Inline -> if (italic) SelectionContainer {
            Text(payload.text, style = MaterialTheme.typography.bodySmall, fontStyle = FontStyle.Italic, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 22.dp, top = 2.dp))
        } else Output(payload.text)
        is BindingMessagePayload.Deferred -> Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
            Text("正在读取 ${bytes(payload.bytes)}…", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp))
        }
    }
}

/** Tool output: monospace, unwrapped, at most a screenful tall. */
@Composable
private fun Output(text: String) {
    val copy = rememberCopy()
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = RoundedCornerShape(8.dp), modifier = Modifier.fillMaxWidth()) {
        Box {
            SelectionContainer {
                Text(
                    text.trimEnd(), style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, softWrap = false,
                    modifier = Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState()).padding(10.dp).padding(end = 28.dp),
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
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHighest, shape = RoundedCornerShape(8.dp)) {
        Row(Modifier.padding(horizontal = 10.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(16.dp))
            Text(label, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp))
        }
    }
}

@Composable
private fun Working() {
    Row(verticalAlignment = Alignment.CenterVertically) {
        CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp)
        Text("正在工作…", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp))
    }
}

private fun bytes(count: ULong): String = when {
    count < 1_024uL -> "$count B"
    count < 1_048_576uL -> "${count / 1_024uL} KB"
    else -> "%.1f MB".format(count.toDouble() / 1_048_576.0)
}
