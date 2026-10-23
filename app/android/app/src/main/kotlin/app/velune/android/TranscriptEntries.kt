package app.velune.android

import app.velune.bindings.BindingMessage
import app.velune.bindings.BindingMessageBlock
import app.velune.bindings.BindingToolState
import app.velune.bindings.BindingTranscriptItem
import app.velune.bindings.BindingTranscriptTurn

/** One row of the transcript: a message, or a turn's work folded into one line. */
sealed interface TranscriptEntry {
    val key: String

    data class Message(override val key: String, val message: BindingMessage) : TranscriptEntry

    /** The reasoning, tool calls and intermediate text between a user message and the reply. */
    data class Work(override val key: String, val members: List<BindingMessage>, val durationMs: ULong?, val running: Boolean) : TranscriptEntry {
        private val tools get() = members.flatMap { it.blocks }.filterIsInstance<BindingMessageBlock.Tool>()
        val toolCount: Int get() = tools.size
        val failedTools: Int get() = tools.count { it.state == BindingToolState.FAILED }
    }
}

/**
 * Lays the core's transcript items out as rows. A reply that shares its
 * message with the turn's last work (`reply:` items) shows only the blocks
 * from `replyStartBlock`; the work group keeps the blocks before it.
 * `message` returns the newest copy of a message, loaded payloads included.
 */
fun transcriptEntries(
    items: List<BindingTranscriptItem>,
    turns: List<BindingTranscriptTurn>,
    message: (String) -> BindingMessage?,
): List<TranscriptEntry> {
    val byTurn = turns.associateBy { it.id }
    val replyStart = turns.filter { it.replyStartBlock > 0u }.mapNotNull { turn -> turn.lastMessageId?.let { it to turn.replyStartBlock.toInt() } }.toMap()
    return items.mapNotNull { item ->
        when (item) {
            is BindingTranscriptItem.Message -> {
                val found = message(item.messageId) ?: return@mapNotNull null
                val start = replyStart[item.messageId]
                val shown = if (item.id.startsWith("reply:") && start != null) found.copy(blocks = found.blocks.drop(start)) else found
                TranscriptEntry.Message(item.id, shown)
            }
            is BindingTranscriptItem.Work -> {
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
