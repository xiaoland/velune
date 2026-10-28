package app.velune.android

import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.conversation.ToolState
import app.velune.bindings.conversation.TranscriptItem
import app.velune.bindings.conversation.TranscriptTurn

/** One row of the transcript: a message, or a turn's work folded into one line. */
sealed interface TranscriptEntry {
    val key: String

    data class Message(override val key: String, val message: DeliveredMessage) : TranscriptEntry

    /** The reasoning, tool calls and intermediate text between a user message and the reply. */
    data class Work(override val key: String, val members: List<DeliveredMessage>, val durationMs: ULong?, val running: Boolean) : TranscriptEntry {
        private val tools get() = members.flatMap { it.blocks }.filterIsInstance<DeliveredBlock.Tool>()
        val toolCount: Int get() = tools.size
        val failedTools: Int get() = tools.count { it.state == ToolState.FAILED }
    }
}

/**
 * Lays the core's transcript items out as rows. A reply that shares its
 * message with the turn's last work (`reply:` items) shows only the blocks
 * from `replyStartBlock`; the work group keeps the blocks before it.
 * `message` returns the newest copy of a message, loaded payloads included.
 */
fun transcriptEntries(
    items: List<TranscriptItem>,
    turns: List<TranscriptTurn>,
    message: (String) -> DeliveredMessage?,
): List<TranscriptEntry> {
    val byTurn = turns.associateBy { it.id }
    val replyStart = turns.filter { it.replyStartBlock > 0u }.mapNotNull { turn -> turn.lastMessageId?.let { it to turn.replyStartBlock.toInt() } }.toMap()
    return items.mapNotNull { item ->
        when (item) {
            is TranscriptItem.Message -> {
                val found = message(item.messageId) ?: return@mapNotNull null
                val start = replyStart[item.messageId]
                val shown = if (item.id.startsWith("reply:") && start != null) found.copy(blocks = found.blocks.drop(start)) else found
                TranscriptEntry.Message(item.id, shown)
            }
            is TranscriptItem.Work -> {
                val turn = byTurn[item.turnId] ?: return@mapNotNull null
                val members = turn.workMessageIds.mapNotNull { id ->
                    val found = message(id) ?: return@mapNotNull null
                    val start = replyStart[id]
                    if (start != null && id == turn.lastMessageId) found.copy(blocks = found.blocks.take(start)) else found
                }.filter { it.blocks.isNotEmpty() }
                if (members.isEmpty()) null else TranscriptEntry.Work("work:" + turn.id, members, turn.durationMs, turn.isRunning)
            }
        }
    }
}
