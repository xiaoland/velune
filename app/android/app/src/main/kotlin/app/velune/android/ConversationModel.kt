package app.velune.android

import app.velune.bindings.application.ComposerAttachment
import app.velune.bindings.application.ComposerInput
import app.velune.bindings.application.ComposerInputContent
import app.velune.bindings.application.ComposerInputOperation
import app.velune.bindings.application.ComposerContextKey
import app.velune.bindings.application.ComposerTargetChoices
import app.velune.bindings.application.ComposerTargetRequest
import app.velune.bindings.application.PreparedComposerTarget
import app.velune.bindings.application.NextTurn
import app.velune.bindings.application.resolveComposerTarget
import app.velune.bindings.application.normalizeComposerTargetChoices
import app.velune.bindings.application.ComposerRuntimeChoice
import app.velune.bindings.application.TurnChoice
import java.util.UUID
import app.velune.bindings.application.ComposedTurn
import app.velune.bindings.application.DeliveredSnapshot
import app.velune.bindings.conversation.ConversationSummary
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.conversation.ModelSelection
import app.velune.bindings.conversation.InputAccess
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteraction
import app.velune.bindings.conversation.RuntimeInteractionReply
import app.velune.bindings.application.RuntimeModel
import app.velune.bindings.application.SnapshotResult
import app.velune.bindings.application.SnapshotUpdate
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
    val conversation: ConversationSummary? = null,
    val runState: RunState = RunState.IDLE,
    val entries: List<TranscriptEntry> = emptyList(),
    val interactions: List<RuntimeInteraction> = emptyList(),
    val canSend: Boolean = true,
    val inputAccess: InputAccess = InputAccess.EDITABLE,
    val canCancel: Boolean = false,
    val canCompact: Boolean = false,
    val models: List<RuntimeModel> = emptyList(),
    /** None: the runtime's own default. */
    val selection: ModelSelection? = null,
    val busy: Boolean = false,
    val loading: Boolean = true,
    val failure: String? = null,
    /** The text just sent, shown until the device's copy of it arrives. */
    val sending: String? = null,
    val draft: String = "",
    val attachments: List<ComposerAttachment> = emptyList(),
    val composerReady: Boolean = false,
    val composerFailure: String? = null,
) {
    val running: Boolean get() = runState == RunState.RUNNING || runState == RunState.STOPPING
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
    private var composer: ComposerInput? = null
    private var composerOwner: String? = null
    private var composerRevision: ULong? = null
    private var composerBaseline: ULong? = null
    private var composerPending = false
    private var contentGeneration = 0L
    private var targetChoices = ComposerTargetChoices(null, null, emptyList(), false)
    private var preparedComposerTarget: PreparedComposerTarget? = null
    private var targetRevision: ULong? = null
    private var targetPending = false
    private var targetBaseline: ULong? = null
    private var targetWriteFailed = false
    private var pendingCreation: PendingComposerCreation? = null
    private var composerUnbound = false
    private var composerRestored = false
    private var composerCacheBlocked = false
    private var composerBindingRevision: ULong? = null
    private var composerBindingBaseline: ULong? = null
    private var composerLoaded = false
    private var composerSaving = false
    private var composerPolling = false
    private var composerCatalogRuntime: String? = null
    private var composerSave: Job? = null
    private val composerKey: String get() = peer + "/" + ((target as? ConversationTarget.Existing)?.conversationId ?: "new")
    private var snapshot: DeliveredSnapshot? = null
    private var messages: Map<String, DeliveredMessage> = emptyMap()
    /** Deferred payloads read on request, by message ID. */
    private var payloads: Map<String, DeliveredMessage> = emptyMap()

    init {
        runCatching { hub.readComposer(composerKey) }.onSuccess { saved ->
            if (saved != null) {
                composer = saved.input; composerPending = saved.pending; composerBaseline = saved.baseline; composerUnbound = saved.unbound; composerRevision = saved.directoryRevision; pendingCreation = saved.creation; composerRestored = true; composerBindingRevision = saved.bindingRevision; composerBindingBaseline = saved.bindingBaseline
                targetChoices = normalizeComposerTargetChoices(saved.target ?: targetChoices); targetRevision = saved.targetRevision; targetPending = saved.targetPending; targetBaseline = saved.targetBaseline
                _state.update { it.copy(draft = saved.input.content.text, attachments = saved.input.content.attachments, selection = targetChoices.choices.firstOrNull { choice -> choice.runtimeId == target.runtimeId }?.choice?.model) }
            }
        }.onFailure { error -> composerCacheBlocked = true; _state.update { it.copy(composerFailure = composerError(error)) } }
        scope.launch { pollComposer() }
        scope.launch {
            val runtime = target.runtimeId
            val models = runCatching { io(NativeOperation.CONFIGURATION_READ) { it.runtimeModels(runtime) } }.getOrDefault(emptyList())
            if ((targetChoices.runtimeId ?: snapshot?.contextRuntimeId ?: target.runtimeId) == runtime) {
                composerCatalogRuntime = runtime
                _state.update { it.copy(models = models) }
            }
        }
        val opening = initial as? ConversationTarget.Existing
        if (opening != null) scope.launch {
            try {
                apply(io(NativeOperation.CONVERSATION_READ) { it.readConversation(opening.runtimeId, opening.conversationId) })
                pollComposer()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _state.update { it.copy(failure = error.describe()) }
            } finally {
                _state.update { it.copy(loading = false) }
            }
        }
    }

    private suspend fun <T> io(operation: NativeOperation, body: (VeluneApplication) -> T): T = hub.call(peer, operation, body)

    fun resume() {
        if (polling?.isActive == true) return
        polling = scope.launch {
            while (isActive) {
                delay(if (_state.value.running) 700 else 3_000)
                pollComposer()
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
            when (val update = io(NativeOperation.CONVERSATION_READ) { it.snapshotUpdate(runtime, id, current.cursor) }) {
                is SnapshotUpdate.Changed -> merge(update.snapshot)
                SnapshotUpdate.Unchanged -> {}
                // The device let the conversation go (it restarted, for one); read it again.
                SnapshotUpdate.Absent -> apply(io(NativeOperation.CONVERSATION_READ) { it.readConversation(runtime, id) })
            }
            _state.update { it.copy(failure = null) }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _state.update { it.copy(failure = error.describe()) }
        }
    }

    private fun apply(result: SnapshotResult) {
        result.snapshot?.let(::merge)
    }

    /** A snapshot with a base holds only changed messages; `messageIds` gives the whole order. */
    private fun merge(next: DeliveredSnapshot) {
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
                inputAccess = next.actions.inputAccess,
                canCompact = next.followUp.canCompact,
                canCancel = next.actions.canCancel,
                selection = state.selection,
            )
        }
        val conversation = next.conversation
        target = ConversationTarget.Existing(next.contextRuntimeId, conversation.id, conversation.title.displayText())
    }

    private fun entries(of: DeliveredSnapshot) =
        transcriptEntries(of.transcriptItems, of.transcriptTurns) { id -> payloads[id] ?: messages[id] }

    fun select(selection: ModelSelection?) {
        if (!_state.value.composerReady || _state.value.busy) return
        val runtime = targetChoices.runtimeId ?: target.runtimeId
        val previous = targetChoices.choices.firstOrNull { it.runtimeId == runtime && it.ownerId == composerOwner }?.choice ?: TurnChoice(model = null, effort = null, permissionMode = null)
        targetBaseline = targetRevision
        targetChoices = targetChoices.copy(choices = targetChoices.choices.filter { it.runtimeId != runtime || it.ownerId != composerOwner } + ComposerRuntimeChoice(composerOwner ?: return, runtime, previous.copy(model = selection)), choiceEdited = true)
        targetPending = true
        _state.update { it.copy(selection = selection) }; cacheComposer()
        scope.launch { runCatching { flushComposer() }.onFailure { error -> _state.update { it.copy(composerFailure = composerError(error)) } } }
    }
    fun editDraft(text: String) { _state.update { it.copy(draft = text) }; composerChanged() }
    fun removeAttachment(id: String) { _state.update { it.copy(attachments = it.attachments.filter { file -> file.id != id }) }; composerChanged() }
    private fun content(): ComposerInputContent {
        return ComposerInputContent(_state.value.draft, _state.value.attachments)
    }
    private fun cacheComposer() { if (composerCacheBlocked) return; composer?.let { hub.saveComposer(composerKey, SavedComposerInput(it.copy(content = content()), composerPending, composerBaseline, composerUnbound, composerRevision, pendingCreation, composerBindingRevision, composerBindingBaseline, targetChoices, targetRevision, targetPending, targetBaseline)) } }
    private fun composerTargetRequest() = ComposerTargetRequest(snapshot?.conversation?.runtimeId ?: target.runtimeId, snapshot?.conversation?.id ?: (target as? ConversationTarget.Existing)?.conversationId)
    private fun effectiveNextTurn(): NextTurn? = preparedComposerTarget?.let { prepared ->
        if (prepared.request != composerTargetRequest() || snapshot?.let { it.contextRuntimeId != prepared.defaultRuntimeId } == true) return@let null
        resolveComposerTarget(prepared, targetChoices, _state.value.models.takeIf { composerCatalogRuntime == prepared.basis.runtimeId })
    }
    private fun targetContext(owner: String) = (target as? ConversationTarget.Existing)?.let { ComposerContextKey.Conversation(owner, it.conversationId) } ?: ComposerContextKey.NewConversation
    private fun composerError(error: Throwable) = error.describe() + if (targetWriteFailed) "；重新选择以采用当前配置" else ""
    private fun composerChanged() {
        contentGeneration++
        if (!composerPending) composerBaseline = composer?.revision
        composerPending = true
        if (composer == null) { composer = ComposerInput(UUID.randomUUID().toString(), 0u, content()); composerUnbound = true; composerBindingBaseline = composerBindingRevision }
        cacheComposer()
        composerSave?.cancel()
        composerSave = scope.launch {
            delay(250)
            try { flushComposer() } catch (error: CancellationException) { throw error } catch (error: Exception) { _state.update { it.copy(composerFailure = composerError(error)) } }
        }
    }
    private suspend fun flushComposer() {
        if (composerSaving || !composerLoaded) return
        composerSaving = true
        try {
            while (composerPending) {
                val input = composer ?: return
                val sent = content()
                val update = io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Save(input.id, composerBaseline?.takeUnless { revision -> revision == 0uL }, sent)) }
                val saved = update.state?.inputs?.firstOrNull { it.id == input.id } ?: error("设备没有返回输入")
                composer = saved; composerBaseline = saved.revision
                if (content() == sent) { composerPending = false; composerBaseline = null }
                cacheComposer()
            }
            while (targetPending) {
                val owner = composerOwner ?: error("目标来源尚未恢复")
                val choices = targetChoices
                val saved = try { io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.SaveTarget(targetContext(owner), targetBaseline, choices)) }.state?.targets?.firstOrNull { it.context == targetContext(owner) } ?: error("设备没有返回目标配置") }
                catch (error: Exception) { targetWriteFailed = true; throw error }
                targetWriteFailed = false
                targetRevision = saved.revision; targetBaseline = saved.revision
                if (targetChoices == choices) { targetPending = false; targetBaseline = null }
                cacheComposer()
            }
            if (composerUnbound) {
                val owner = composerOwner ?: error("输入来源尚未恢复")
                val context = (target as? ConversationTarget.Existing)?.let { ComposerContextKey.Conversation(owner, it.conversationId) } ?: ComposerContextKey.NewConversation
                val inputId = composer!!.id
                io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Bind(context, inputId, composerBindingBaseline)) }
                composerUnbound = false; cacheComposer()
            }
        } finally { composerSaving = false }
    }
    private suspend fun completeCreation() {
        val creation = pendingCreation ?: return
        check(creation.ownerId == composerOwner) { "新会话的执行来源不可用，输入已保留" }
        val oldKey = composerKey
        if (snapshot == null) {
            val created = io(NativeOperation.CONVERSATION_READ) { it.readConversation(creation.runtimeId, creation.conversationId) }
            apply(created); target = ConversationTarget.Existing(creation.runtimeId, creation.conversationId, "会话")
        }
        creation.choices?.let { choices ->
            val context = ComposerContextKey.Conversation(creation.ownerId, creation.conversationId)
            val updated = io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.EnsureTarget(context, choices)) }
            val actual = updated.state?.targets?.firstOrNull { it.context == context } ?: error("设备没有返回新会话的目标配置")
            targetRevision = actual.revision
            if (targetPending) targetBaseline = actual.revision.takeIf { normalizeComposerTargetChoices(actual.choices) == normalizeComposerTargetChoices(choices) }
            else observeTarget(actual, creation.ownerId)
        }
        preparedComposerTarget = null
        io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Promote(creation.inputId, creation.ownerId, creation.conversationId, creation.runtimeId)) }
        pendingCreation = null; cacheComposer()
        val newKey = peer + "/new"
        if (hub.readComposer(newKey)?.creation?.inputId == creation.inputId) hub.removeComposer(newKey)
        if (oldKey != composerKey && hub.readComposer(oldKey)?.input?.id == creation.inputId) hub.removeComposer(oldKey)
    }
    private fun observeTarget(observed: app.velune.bindings.application.ComposerTarget, owner: String) {
        targetRevision = observed.revision
        val received = normalizeComposerTargetChoices(observed.choices)
        if (targetPending && normalizeComposerTargetChoices(targetChoices) == received) { targetPending = false; targetBaseline = null; targetWriteFailed = false }
        if (!targetPending) {
            targetChoices = received
            val runtime = targetChoices.runtimeId ?: preparedComposerTarget?.defaultRuntimeId ?: snapshot?.contextRuntimeId ?: target.runtimeId
            _state.update { it.copy(selection = targetChoices.choices.firstOrNull { choice -> choice.runtimeId == runtime && choice.ownerId == owner }?.choice?.model) }
        }
    }
    private suspend fun pollComposer() {
        if (composerCacheBlocked || _state.value.busy || composerPolling || composerSaving || target is ConversationTarget.Existing && snapshot == null) return
        composerPolling = true
        try {
            val owner = composerOwner ?: io(NativeOperation.CONVERSATION_READ) { it.composerOwnerId() }.also { composerOwner = it }
            completeCreation()
            val request = composerTargetRequest()
            val context = targetContext(owner)
            val baseline = composerRevision ?: 0uL
            val update = io(NativeOperation.CONVERSATION_READ) { it.manageComposerInputs(ComposerInputOperation.Read(composerRevision)) }
            if (request != composerTargetRequest()) return
            if (update.state != null) {
                var directory = checkNotNull(update.state)
                val creation = directory.creations.firstOrNull { creation -> creation.inputId == composer?.id && creation.ownerId == owner && creation.revision > baseline && directory.bindings.any { it.inputId == creation.inputId && it.context == ComposerContextKey.Conversation(owner, creation.conversationId) } }
                if (target is ConversationTarget.New && creation != null) {
                    composerLoaded = false
                    _state.update { it.copy(composerReady = false) }
                    val oldKey = composerKey
                    val created = io(NativeOperation.CONVERSATION_READ) { it.readConversation(creation.runtimeId, creation.conversationId) }
                    apply(created); target = ConversationTarget.Existing(creation.runtimeId, creation.conversationId, "会话")
                    preparedComposerTarget = null
                    cacheComposer(); hub.removeComposer(oldKey)
                    return
                }
                if (directory.bindings.none { it.context == context }) {
                    directory = io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Ensure(context, UUID.randomUUID().toString(), ComposerInputContent("", emptyList()))) }.state ?: error("设备没有返回输入目录")
                    if (request != composerTargetRequest()) return
                }
                composerRevision = directory.revision
                val binding = directory.bindings.first { it.context == context }
                val inputId = binding.inputId
                composerBindingRevision = binding.revision
                if (composerUnbound && inputId == composer?.id) { composerUnbound = false; composerBindingBaseline = null }
                else if (composerUnbound && !composerRestored && composer?.revision == 0uL) composerBindingBaseline = binding.revision
                if (!composerPending && !composerUnbound) {
                    val input = directory.inputs.first { it.id == inputId }
                    composer = input
                    _state.update { it.copy(draft = input.content.text, attachments = input.content.attachments) }
                    cacheComposer()
                }
                directory.targets.firstOrNull { it.context == context }?.let { observeTarget(it, owner) }
            }
            composerLoaded = true
            val prepared = io(NativeOperation.CONVERSATION_WRITE) { it.prepareComposerTarget(request, targetChoices.takeIf { targetPending }) }
            if (request != composerTargetRequest()) return
            check(prepared.ownerId == owner) { "执行端身份已改变，请重新打开会话，输入已保留" }
            preparedComposerTarget = prepared
            observeTarget(prepared.target, owner)
            check(targetChoices.runtimeOwnerId == null || targetChoices.runtimeOwnerId == owner) { "所选运行时位于另一设备，输入已保留" }
            val selectedRuntime = targetChoices.runtimeId ?: prepared.defaultRuntimeId
            val catalog = if (composerCatalogRuntime == selectedRuntime) _state.value.models else io(NativeOperation.CONFIGURATION_READ) { it.runtimeModels(selectedRuntime) }
            if (request != composerTargetRequest()) return
            composerCatalogRuntime = selectedRuntime
            val selection = _state.value.selection
            check(selection == null || catalog.any { it.selection == selection }) { "所选模型在此设备不可用，输入已保留" }
            val ready = (snapshot == null || snapshot?.contextRuntimeId == prepared.defaultRuntimeId)
                && resolveComposerTarget(prepared, targetChoices, catalog)?.chosenModelUnavailable == false
            _state.update { it.copy(models = catalog, composerReady = ready) }
            flushComposer()
            _state.update { it.copy(composerFailure = null) }
        } catch (error: CancellationException) { throw error } catch (error: Exception) { _state.update { it.copy(composerReady = false, composerFailure = composerError(error)) } }
        finally { composerPolling = false }
    }

    fun send(text: String, onSent: () -> Unit) {
        val submitted = content()
        val submittedGeneration = contentGeneration
        val submittedTarget = targetChoices
        val next = effectiveNextTurn() ?: return
        val selection = next.model
        act(sending = text) {
        if (!_state.value.composerReady || pendingCreation != null || _state.value.composerFailure != null) return@act
        flushComposer()
        val input = composer ?: return@act
        val runtime = next.runtimeId
        val turn = ComposedTurn(text = text, effort = next.effort, permissionMode = next.permissionMode?.id, attachments = submitted.attachments.map { "composer:" + it.id })
        val existing = snapshot
        val result = if (existing == null) {
            val created = io(NativeOperation.CONVERSATION_WRITE) { it.createConversation(runtime, (target as? ConversationTarget.New)?.cwd, selection) }
            val oldKey = composerKey
            val snapshot = created.snapshot ?: error("设备没有返回新会话")
            val owner = composerOwner ?: error("输入来源尚未恢复")
            pendingCreation = PendingComposerCreation(input.id, owner, snapshot.conversation.runtimeId, snapshot.conversation.id, submittedTarget)
            _state.update { it.copy(composerReady = false) }
            cacheComposer()
            apply(created); target = ConversationTarget.Existing(snapshot.contextRuntimeId, snapshot.conversation.id, "新会话")
            cacheComposer()
            completeCreation()
            _state.update { it.copy(composerReady = false) }
            if (hub.readComposer(oldKey)?.input?.id == input.id) hub.removeComposer(oldKey)
            io(NativeOperation.SEND) { it.sendTurn(snapshot.contextRuntimeId, snapshot.conversation.id, runtime, selection, turn) }
        } else {
            io(NativeOperation.SEND) { it.sendTurn(existing.contextRuntimeId, existing.conversation.id, runtime, selection, turn) }
        }
        apply(result)
        if (composer?.id == input.id && contentGeneration == submittedGeneration && content() == submitted) {
            _state.update { it.copy(draft = "", attachments = emptyList()) }; composerChanged(); flushComposer()
        }
        onSent()
        }
    }

    fun compact() = act {
        val current = snapshot ?: return@act
        if (!current.followUp.canCompact) return@act
        val selection = state.value.selection
        apply(io(NativeOperation.COMPACT) { it.compactConversation(current.contextRuntimeId, current.conversation.id, selection) })
    }

    fun stop() = act {
        val current = snapshot ?: return@act
        apply(io(NativeOperation.CANCEL) { it.cancel(current.contextRuntimeId, current.conversation.id) })
    }

    fun reply(interactionId: String, reply: RuntimeInteractionReply) = act {
        val current = snapshot ?: return@act
        apply(io(NativeOperation.REPLY) { it.replyRuntimeInteraction(current.contextRuntimeId, current.conversation.id, interactionId, reply) })
    }

    /** Large reasoning and tool output arrive by size; this reads one message's whole content. */
    fun loadPayload(messageId: String) = scope.launch {
        val current = snapshot ?: return@launch
        try {
            val loaded = io(NativeOperation.CONVERSATION_READ) { it.messagePayloads(current.contextRuntimeId, current.conversation.id, listOf(messageId)) }
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
                if (isActive) pollComposer()
            }
        }
    }
}
