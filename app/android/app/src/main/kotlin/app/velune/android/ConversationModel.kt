package app.velune.android

import app.velune.bindings.BindingComposedTurn
import app.velune.bindings.BindingConversationSnapshot
import app.velune.bindings.BindingConversationSummary
import app.velune.bindings.BindingMessage
import app.velune.bindings.BindingModelSelection
import app.velune.bindings.BindingRunState
import app.velune.bindings.BindingRuntimeInteraction
import app.velune.bindings.BindingRuntimeInteractionReply
import app.velune.bindings.BindingRuntimeModel
import app.velune.bindings.BindingSnapshotResult
import app.velune.bindings.BindingSnapshotUpdate
import app.velune.bindings.VeluneApplication
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Where a conversation screen points: an existing conversation, or one to start on a runtime. */
sealed interface ConversationTarget {
    val runtimeId: String
    data class Existing(override val runtimeId: String, val conversationId: String, val title: String) : ConversationTarget
    data class New(override val runtimeId: String, val cwd: String?) : ConversationTarget
}

data class ConversationState(
    val conversation: BindingConversationSummary? = null,
    val runState: BindingRunState = BindingRunState.IDLE,
    val entries: List<TranscriptEntry> = emptyList(),
    val interactions: List<BindingRuntimeInteraction> = emptyList(),
    val canSend: Boolean = true,
    val canCancel: Boolean = false,
    val models: List<BindingRuntimeModel> = emptyList(),
    /** None: the runtime's own default. */
    val selection: BindingModelSelection? = null,
    val busy: Boolean = false,
    val loading: Boolean = true,
    val failure: String? = null,
    /** The text just sent, shown until the device's copy of it arrives. */
    val sending: String? = null,
) {
    val running: Boolean get() = runState == BindingRunState.RUNNING || runState == BindingRunState.STOPPING
}

/** One open conversation on a controlled device. */
class ConversationModel(
    private val hub: DeviceHub,
    private val peer: String,
    initial: ConversationTarget,
    private val scope: CoroutineScope,
) {
    var target: ConversationTarget = initial
        private set
    private val _state = MutableStateFlow(ConversationState(loading = initial is ConversationTarget.Existing))
    val state: StateFlow<ConversationState> = _state.asStateFlow()
    private var polling: Job? = null
    private var snapshot: BindingConversationSnapshot? = null
    private var messages: Map<String, BindingMessage> = emptyMap()
    /** Deferred payloads read on request, by message ID. */
    private var payloads: Map<String, BindingMessage> = emptyMap()

    init {
        scope.launch {
            val models = runCatching { io { it.runtimeModels(target.runtimeId) } }.getOrDefault(emptyList())
            _state.update { it.copy(models = models) }
        }
        val opening = initial as? ConversationTarget.Existing
        if (opening != null) scope.launch {
            try {
                apply(io { it.readConversation(opening.runtimeId, opening.conversationId) })
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(failure = error.describe()) }
            } finally {
                _state.update { it.copy(loading = false) }
            }
        }
    }

    private suspend fun <T> io(operation: String = "read", body: (VeluneApplication) -> T): T = hub.call(peer, operation, body)

    fun resume() {
        if (polling?.isActive == true) return
        polling = scope.launch {
            while (isActive) {
                delay(if (_state.value.running) 700 else 3_000)
                poll()
            }
        }
    }

    fun pause() {
        polling?.cancel()
        polling = null
    }

    private suspend fun poll() {
        val current = snapshot ?: return
        val runtime = current.contextRuntimeId
        val id = current.conversation.id
        try {
            when (val update = io { it.snapshotUpdate(runtime, id, current.cursor) }) {
                is BindingSnapshotUpdate.Changed -> merge(update.snapshot)
                BindingSnapshotUpdate.Unchanged -> {}
                // The device let the conversation go (it restarted, for one); read it again.
                BindingSnapshotUpdate.Absent -> apply(io { it.readConversation(runtime, id) })
            }
            _state.update { it.copy(failure = null) }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _state.update { it.copy(failure = error.describe()) }
        }
    }

    private fun apply(result: BindingSnapshotResult) {
        result.snapshot?.let(::merge)
    }

    /** A snapshot with a base holds only changed messages; `messageIds` gives the whole order. */
    private fun merge(next: BindingConversationSnapshot) {
        val fresh = next.messages.associateBy { it.id }
        val order = next.messageIds.toSet()
        messages = if (next.base == null) fresh else (messages + fresh).filterKeys { it in order }
        // A loaded payload goes stale once its message changes again.
        payloads = payloads.filter { (id, loaded) -> fresh[id]?.let { it.stamp == loaded.stamp } ?: (id in order) }
        snapshot = next
        _state.update { state ->
            state.copy(
                conversation = next.conversation,
                runState = next.runState,
                entries = entries(next),
                interactions = next.pendingInteractions,
                canSend = next.actions.canSend,
                canCancel = next.actions.canCancel,
                selection = state.selection ?: next.modelRecordKey?.let { BindingModelSelection.Managed(it) },
            )
        }
        val conversation = next.conversation
        target = ConversationTarget.Existing(next.contextRuntimeId, conversation.id, conversation.title)
    }

    private fun entries(of: BindingConversationSnapshot) =
        transcriptEntries(of.transcriptItems, of.transcriptTurns) { id -> payloads[id] ?: messages[id] }

    fun select(selection: BindingModelSelection?) = _state.update { it.copy(selection = selection) }

    fun send(text: String, onSent: () -> Unit) = act(sending = text) {
        val turn = BindingComposedTurn(text = text, effort = null, permissionMode = null, attachments = emptyList())
        val selection = _state.value.selection
        val existing = snapshot
        val result = if (existing == null) {
            val created = io { it.createConversationWithSelection(target.runtimeId, (target as? ConversationTarget.New)?.cwd, selection) }
            apply(created)
            val snapshot = created.snapshot ?: error("设备没有返回新会话")
            io("send") { it.sendTurnWithSelection(snapshot.contextRuntimeId, snapshot.conversation.id, snapshot.contextRuntimeId, selection, turn) }
        } else {
            io("send") { it.sendTurnWithSelection(existing.contextRuntimeId, existing.conversation.id, existing.contextRuntimeId, selection, turn) }
        }
        apply(result)
        onSent()
    }

    fun stop() = act {
        val current = snapshot ?: return@act
        apply(io("cancel") { it.cancel(current.contextRuntimeId, current.conversation.id) })
    }

    fun reply(interactionId: String, reply: BindingRuntimeInteractionReply) = act {
        val current = snapshot ?: return@act
        apply(io { it.replyRuntimeInteraction(current.contextRuntimeId, current.conversation.id, interactionId, reply) })
    }

    /** Large reasoning and tool output arrive by size; this reads one message's whole content. */
    fun loadPayload(messageId: String) = scope.launch {
        val current = snapshot ?: return@launch
        try {
            val loaded = io { it.messagePayloads(current.contextRuntimeId, current.conversation.id, listOf(messageId)) }
            payloads = payloads + loaded.associateBy { it.id }
            snapshot?.let { shown -> _state.update { it.copy(entries = entries(shown)) } }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _state.update { it.copy(failure = error.describe()) }
        }
    }

    private fun act(sending: String? = null, body: suspend () -> Unit) {
        if (_state.value.busy) return
        _state.update { it.copy(busy = true, failure = null, sending = sending) }
        scope.launch {
            try {
                body()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(failure = error.describe()) }
            } finally {
                _state.update { it.copy(busy = false, sending = null) }
            }
        }
    }
}
