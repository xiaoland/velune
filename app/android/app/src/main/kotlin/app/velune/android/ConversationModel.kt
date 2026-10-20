package app.velune.android

import app.velune.bindings.BindingComposedTurn
import app.velune.bindings.BindingConversationSnapshot
import app.velune.bindings.BindingMessage
import app.velune.bindings.BindingModelSelection
import app.velune.bindings.BindingRunState
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
    val snapshot: BindingConversationSnapshot? = null,
    val messages: List<BindingMessage> = emptyList(),
    val models: List<BindingRuntimeModel> = emptyList(),
    /** None: the runtime's own default. */
    val selection: BindingModelSelection? = null,
    val busy: Boolean = false,
    val loading: Boolean = true,
    val failure: String? = null,
    /** Deferred payloads read on request, by message ID. */
    val payloads: Map<String, BindingMessage> = emptyMap(),
) {
    val running: Boolean get() = snapshot?.runState == BindingRunState.RUNNING || snapshot?.runState == BindingRunState.STOPPING
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

    private suspend fun <T> io(body: (VeluneApplication) -> T): T {
        val application = hub.application(peer)
        return withContext(Dispatchers.IO) { body(application) }
    }

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
        val current = _state.value.snapshot ?: return
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
    private fun merge(snapshot: BindingConversationSnapshot) {
        _state.update { state ->
            val messages = if (snapshot.base == null) snapshot.messages else {
                val known = state.messages.associateBy { it.id } + snapshot.messages.associateBy { it.id }
                snapshot.messageIds.mapNotNull { known[it] }
            }
            val selection = state.selection ?: snapshot.modelRecordKey?.let { BindingModelSelection.Managed(it) }
            state.copy(snapshot = snapshot, messages = messages, selection = selection)
        }
        val conversation = snapshot.conversation
        target = ConversationTarget.Existing(snapshot.contextRuntimeId, conversation.id, conversation.title)
    }

    fun select(selection: BindingModelSelection?) = _state.update { it.copy(selection = selection) }

    fun send(text: String, onSent: () -> Unit) = act {
        val turn = BindingComposedTurn(text = text, effort = null, permissionMode = null, attachments = emptyList())
        val selection = _state.value.selection
        val existing = _state.value.snapshot
        val result = if (existing == null) {
            val created = io { it.createConversationWithSelection(target.runtimeId, (target as? ConversationTarget.New)?.cwd, selection) }
            apply(created)
            val snapshot = created.snapshot ?: error("设备没有返回新会话")
            io { it.sendTurnWithSelection(snapshot.contextRuntimeId, snapshot.conversation.id, snapshot.contextRuntimeId, selection, turn) }
        } else {
            io { it.sendTurnWithSelection(existing.contextRuntimeId, existing.conversation.id, existing.contextRuntimeId, selection, turn) }
        }
        apply(result)
        onSent()
    }

    fun stop() = act {
        val snapshot = _state.value.snapshot ?: return@act
        apply(io { it.cancel(snapshot.contextRuntimeId, snapshot.conversation.id) })
    }

    fun reply(interactionId: String, reply: BindingRuntimeInteractionReply) = act {
        val snapshot = _state.value.snapshot ?: return@act
        apply(io { it.replyRuntimeInteraction(snapshot.contextRuntimeId, snapshot.conversation.id, interactionId, reply) })
    }

    /** Large reasoning and tool output arrive by size; this reads one message's whole content. */
    fun loadPayload(messageId: String) = scope.launch {
        val snapshot = _state.value.snapshot ?: return@launch
        try {
            val loaded = io { it.messagePayloads(snapshot.contextRuntimeId, snapshot.conversation.id, listOf(messageId)) }
            _state.update { state -> state.copy(payloads = state.payloads + loaded.associateBy { it.id }) }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _state.update { it.copy(failure = error.describe()) }
        }
    }

    private fun act(body: suspend () -> Unit) {
        if (_state.value.busy) return
        _state.update { it.copy(busy = true, failure = null) }
        scope.launch {
            try {
                body()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(failure = error.describe()) }
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }
}
