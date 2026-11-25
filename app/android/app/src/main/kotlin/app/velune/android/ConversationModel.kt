package app.velune.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredImageSource
import app.velune.bindings.application.DeliveredSnapshot
import app.velune.bindings.application.MessagePayload
import app.velune.bindings.application.PermissionMode
import java.util.Base64
import app.velune.bindings.conversation.ControlHandoff
import app.velune.bindings.conversation.ConversationSummary
import app.velune.bindings.conversation.EditPlacement
import app.velune.bindings.conversation.EditableMessage
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.conversation.ModelSelection
import app.velune.bindings.conversation.InputAccess
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteraction
import app.velune.bindings.conversation.RuntimeInteractionReply
import app.velune.bindings.application.RuntimeModel
import app.velune.bindings.application.SnapshotResult
import app.velune.bindings.application.SnapshotUpdate
import app.velune.bindings.application.SendQueueOperation
import app.velune.bindings.conversation.BackgroundTaskList
import app.velune.bindings.conversation.BackgroundTaskOutput
import app.velune.bindings.conversation.BackgroundTaskOutputStatus
import app.velune.bindings.conversation.BackgroundTaskStatus
import app.velune.bindings.conversation.Collaborator
import app.velune.bindings.conversation.QueuedMessage
import app.velune.bindings.conversation.SubAgent
import app.velune.bindings.BindingException
import app.velune.bindings.VeluneApplication
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Where a conversation screen points: an existing conversation, or one to start on a runtime. */
sealed interface ConversationTarget {
    val runtimeId: String
    data class Existing(override val runtimeId: String, val conversationId: String, val title: String) : ConversationTarget
    data class New(override val runtimeId: String, val cwd: String?, val draftId: String? = null) : ConversationTarget
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
    val controlHandoff: ControlHandoff? = null,
    val models: List<RuntimeModel> = emptyList(),
    /** None: follow the conversation's current model, or the runtime's default. */
    val selection: ModelSelection? = null,
    val hasCurrent: Boolean = false,
    val currentModelName: String? = null,
    val efforts: List<String> = emptyList(),
    val effort: String? = null,
    val currentEffort: String? = null,
    val permissionModes: List<PermissionMode> = emptyList(),
    val permissionMode: String? = null,
    val currentPermissionMode: PermissionMode? = null,
    val editable: Map<String, EditableMessage> = emptyMap(),
    val canFork: Boolean = false,
    val busy: Boolean = false,
    val loading: Boolean = true,
    val failure: String? = null,
    /** The text just sent, shown until the device's copy of it arrives. */
    val sending: String? = null,
    val submittingFollowUp: String? = null,
    val draft: String = "",
    val attachments: List<ComposerAttachment> = emptyList(),
    val composerReady: Boolean = false,
    val composerFailure: String? = null,
    val contextRuntimeId: String = "",
    val canSwitch: Boolean = false,
    val selectedRuntimeId: String = "",
    val switchingRuntime: Boolean = false,
    val uploadingAttachment: Boolean = false,
    val refreshing: Boolean = false,
    val backgroundTasks: BackgroundTaskList? = null,
    val backgroundTaskOutputs: Map<String, BackgroundTaskOutput> = emptyMap(),
    val backgroundTaskOutputFailures: Map<String, String> = emptyMap(),
    val loadingBackgroundTasks: Boolean = false,
    val backgroundTaskFailure: String? = null,
    val loadingBackgroundTaskOutputs: Set<String> = emptySet(),
    val stoppingBackgroundTaskIds: Set<String> = emptySet(),
    val subAgents: List<SubAgent> = emptyList(),
    val collaborators: List<Collaborator> = emptyList(),
    val parentConversationId: String? = null,
    val canStopSubAgents: Boolean = false,
    val collaborationFailure: String? = null,
    val queue: List<QueuedMessage> = emptyList(),
    val canQueue: Boolean = false,
    val canSteer: Boolean = false,
    val steerUsesCurrentModel: Boolean = true,
    val canResume: Boolean = false,
    val isCompacting: Boolean = false,
) {
    val running: Boolean get() = runState == RunState.RUNNING || runState == RunState.STOPPING
    val usesCurrentRuntime: Boolean get() = selectedRuntimeId == contextRuntimeId
    val activeBackgroundTaskCount: Int get() = backgroundTasks?.takeIf { it.live }?.tasks.orEmpty().count {
        it.status == BackgroundTaskStatus.RUNNING || it.status == BackgroundTaskStatus.STOPPING
    }
}

/** One open conversation on a controlled device. */
class ConversationModel(
    private val hub: DeviceHub,
    private val peer: String,
    initial: ConversationTarget,
    private val scope: CoroutineScope,
    initialDraft: String? = null,
) {
    var target: ConversationTarget by mutableStateOf(initial)
        private set
    private val _state = MutableStateFlow(
        ConversationState(loading = initial is ConversationTarget.Existing, selectedRuntimeId = initial.runtimeId),
    )
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
    private val composerLock = Mutex()
    private var composerCatalogRuntime: String? = null
    private var composerSave: Job? = null
    private val composerKey: String get() = peer + "/" + (composerConversationId() ?: "new")
    private var snapshot: DeliveredSnapshot? = null
    private var pollFailure: String? = null
    private val backgroundTaskLock = Mutex()
    private var messages: Map<String, DeliveredMessage> = emptyMap()
    /** Deferred payloads read on request, by message ID. */
    private var payloads: Map<String, DeliveredMessage> = emptyMap()

    init {
        runCatching { hub.readComposer(composerKey) }.onSuccess { saved ->
            if (saved != null) {
                composer = saved.input; composerPending = saved.pending; composerBaseline = saved.baseline; composerUnbound = saved.unbound; composerRevision = saved.directoryRevision; pendingCreation = saved.creation; composerRestored = true; composerBindingRevision = saved.bindingRevision; composerBindingBaseline = saved.bindingBaseline
                targetChoices = normalizeComposerTargetChoices(saved.target ?: targetChoices); targetRevision = saved.targetRevision; targetPending = saved.targetPending; targetBaseline = saved.targetBaseline
                val runtime = selectedRuntimeId()
                _state.update { it.copy(draft = saved.input.content.text, attachments = saved.input.content.attachments, selectedRuntimeId = runtime, selection = targetChoices.choices.firstOrNull { choice -> choice.runtimeId == runtime }?.choice?.model) }
            }
        }.onFailure { error -> composerCacheBlocked = true; _state.update { it.copy(composerFailure = composerError(error)) } }
        if (initialDraft != null && initialDraft != _state.value.draft) editDraft(initialDraft)
        scope.launch { pollComposer() }
        if (initial is ConversationTarget.Existing) scope.launch { readConversation(initial) }
    }

    private suspend fun <T> io(operation: NativeOperation, body: (VeluneApplication) -> T): T = hub.call(peer, operation, body)

    fun resume() {
        if (polling?.isActive == true) return
        polling = scope.launch {
            launch {
                while (isActive) {
                    delay(5_000)
                    if (_state.value.running || _state.value.activeBackgroundTaskCount > 0) {
                        readBackgroundTasks()
                    }
                }
            }
            while (isActive) {
                delay(if (_state.value.running) 700 else 3_000)
                // The input directory and target may take several remote reads.
                launch { pollComposer() }
                poll()
            }
        }
    }

    fun pause() {
        polling?.cancel()
        polling = null
    }

    fun retargetableNewSession(): ConversationTarget.New? =
        (target as? ConversationTarget.New)?.takeIf {
            !_state.value.busy && !_state.value.uploadingAttachment && pendingCreation == null
        }

    fun updateNewSession(next: ConversationTarget.New) {
        val previous = retargetableNewSession() ?: return
        if (next.runtimeId != previous.runtimeId) {
            preparedComposerTarget = null
            composerCatalogRuntime = null
            _state.update {
                it.copy(
                    composerReady = false, models = emptyList(), selection = null, selectedRuntimeId = next.runtimeId,
                    hasCurrent = false, currentModelName = null, efforts = emptyList(), effort = null, currentEffort = null,
                    permissionModes = emptyList(), permissionMode = null, currentPermissionMode = null,
                )
            }
        }
        target = next
        if (next.runtimeId != previous.runtimeId) scope.launch { pollComposer() }
    }

    private suspend fun poll() {
        if (_state.value.busy || _state.value.refreshing) return
        val current = snapshot ?: return
        val runtime = current.contextRuntimeId
        val id = current.conversation.id
        try {
            val update = io(NativeOperation.CONVERSATION_READ) { it.snapshotUpdate(runtime, id, current.cursor) }
            if (snapshot !== current || _state.value.busy || _state.value.refreshing) return
            when (update) {
                is SnapshotUpdate.Changed -> merge(update.snapshot)
                SnapshotUpdate.Unchanged -> {}
                // The device let the conversation go (it restarted, for one); read it again.
                SnapshotUpdate.Absent -> {
                    val result = io(NativeOperation.CONVERSATION_READ) { it.readConversation(runtime, id) }
                    if (snapshot !== current || _state.value.busy || _state.value.refreshing) return
                    apply(result)
                }
            }
            val previousFailure = pollFailure
            if (previousFailure != null) {
                _state.update { if (it.failure == previousFailure) it.copy(failure = null) else it }
                pollFailure = null
            }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            if (snapshot === current && !_state.value.busy && !_state.value.refreshing) {
                val failure = error.describe()
                if (_state.value.failure == null || _state.value.failure == pollFailure) {
                    pollFailure = failure
                    _state.update { it.copy(failure = failure) }
                }
            }
        }
    }

    fun refresh() {
        val existing = target as? ConversationTarget.Existing ?: return
        if (_state.value.refreshing || _state.value.loading || _state.value.busy) return
        _state.update { it.copy(refreshing = true, failure = null) }
        scope.launch {
            try {
                readConversation(existing)
            } finally {
                _state.update { it.copy(refreshing = false) }
            }
        }
    }

    private suspend fun readConversation(existing: ConversationTarget.Existing) {
        val before = snapshot
        val runtime = before?.conversation?.runtimeId ?: existing.runtimeId
        val failurePrefix = if (_state.value.loading) "无法打开会话" else "刷新会话失败"
        try {
            val result = io(NativeOperation.CONVERSATION_READ) {
                it.readConversation(runtime, existing.conversationId)
            }
            if (snapshot === before && !_state.value.busy) {
                merge(result.snapshot ?: error("设备没有返回会话"))
            }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _state.update { it.copy(failure = "$failurePrefix：${error.describe()}") }
        } finally {
            _state.update { it.copy(loading = false) }
        }
        coroutineScope {
            launch { pollComposer(wait = true) }
            readBackgroundTasks()
        }
    }

    private fun apply(result: SnapshotResult) {
        result.snapshot?.let(::merge)
    }

    /** A snapshot with a base holds only changed messages; `messageIds` gives the whole order. */
    private fun merge(next: DeliveredSnapshot) {
        val previous = snapshot
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
                contextRuntimeId = next.contextRuntimeId,
                canSwitch = next.actions.canSwitch,
                selectedRuntimeId = selectedRuntimeId(),
                inputAccess = next.actions.inputAccess,
                canCompact = next.followUp.canCompact,
                controlHandoff = next.followUp.controlHandoff,
                canCancel = next.actions.canCancel,
                editable = next.followUp.editable.associateBy { it.messageId },
                canFork = next.followUp.canFork,
                subAgents = next.subAgents,
                collaborators = next.collaborators,
                parentConversationId = next.conversation.parentConversationId,
                canStopSubAgents = next.followUp.canStopSubAgents,
                queue = next.followUp.queue,
                canQueue = next.followUp.canQueue,
                canSteer = next.followUp.canSteer,
                canResume = next.followUp.canResume,
                isCompacting = next.followUp.isCompacting,
            )
        }
        val conversation = next.conversation
        target = ConversationTarget.Existing(next.contextRuntimeId, conversation.id, conversation.title.displayText())
        when {
            previous?.contextRuntimeId != null && previous.contextRuntimeId != next.contextRuntimeId -> {
                _state.update {
                    it.copy(backgroundTasks = null, backgroundTaskOutputs = emptyMap(),
                        backgroundTaskOutputFailures = emptyMap(), backgroundTaskFailure = null)
                }
                refreshBackgroundTasks()
            }
            previous == null && !_state.value.loading && !_state.value.refreshing -> refreshBackgroundTasks()
            previous?.runState == RunState.RUNNING && next.runState != RunState.RUNNING &&
                !_state.value.refreshing -> refreshBackgroundTasks()
        }
    }

    private fun entries(of: DeliveredSnapshot) =
        transcriptEntries(of.transcriptItems, of.transcriptTurns) { id -> payloads[id] ?: messages[id] }

    private fun selectedRuntimeId(): String =
        choicesForExecution().runtimeId ?: snapshot?.contextRuntimeId ?: target.runtimeId

    fun selectRuntime(runtimeId: String) {
        val current = _state.value
        val owner = composerOwner ?: return
        if (!current.canSwitch || current.busy || current.switchingRuntime) return
        if (current.selectedRuntimeId == runtimeId && current.composerReady) return
        _state.update { it.copy(switchingRuntime = true) }
        scope.launch {
            try {
                composerLock.withLock {
                    if (!_state.value.canSwitch || _state.value.busy) return@withLock
                    targetBaseline = targetRevision
                    targetChoices = targetChoices.copy(runtimeId = runtimeId, runtimeOwnerId = owner)
                    targetPending = true
                    preparedComposerTarget = null
                    composerCatalogRuntime = null
                    _state.update {
                        it.copy(
                            selectedRuntimeId = runtimeId, composerReady = false, composerFailure = null,
                            models = emptyList(), selection = null, hasCurrent = false, currentModelName = null,
                            efforts = emptyList(), effort = null, currentEffort = null, permissionModes = emptyList(),
                            permissionMode = null, currentPermissionMode = null,
                        )
                    }
                    cacheComposer()
                }
                pollComposer(wait = true)
            } finally {
                _state.update { it.copy(switchingRuntime = false) }
            }
        }
    }

    private fun updateTurnChoice(transform: (TurnChoice) -> TurnChoice) {
        if (_state.value.busy || _state.value.switchingRuntime || effectiveNextTurn() == null) return
        val runtime = selectedRuntimeId()
        val owner = composerOwner ?: return
        val previous = targetChoices.choices.firstOrNull { it.runtimeId == runtime && it.ownerId == owner }?.choice ?: TurnChoice(model = null, effort = null, permissionMode = null)
        val updated = transform(previous)
        targetBaseline = targetRevision
        targetChoices = targetChoices.copy(choices = targetChoices.choices.filter { it.runtimeId != runtime || it.ownerId != owner } + ComposerRuntimeChoice(owner, runtime, updated), choiceEdited = true)
        targetPending = true
        applyResolvedNextTurn(updated)
        cacheComposer()
        scope.launch { pollComposer(wait = true) }
    }

    private fun applyResolvedNextTurn(explicit: TurnChoice? = null) {
        val next = effectiveNextTurn()
        val runtime = choicesForExecution().runtimeId ?: preparedComposerTarget?.defaultRuntimeId ?: snapshot?.contextRuntimeId ?: target.runtimeId
        val choice = explicit ?: targetChoices.choices.firstOrNull { it.runtimeId == runtime && it.ownerId == composerOwner }?.choice
        // Android's steer binding can't transmit a model override to the running turn.
        val steerUsesCurrentModel = choice?.model == null || preparedComposerTarget?.let { prepared ->
            val choices = choicesForExecution()
            val inherited = choices.copy(choices = choices.choices.filterNot { it.runtimeId == runtime && it.ownerId == composerOwner })
            val running = resolveComposerTarget(prepared, inherited, _state.value.models)
            running != null && next != null && running.model == next.model
        } == true
        _state.update { state ->
            state.copy(
                models = next?.models ?: state.models,
                selectedRuntimeId = selectedRuntimeId(),
                selection = choice?.model,
                steerUsesCurrentModel = steerUsesCurrentModel,
                hasCurrent = next?.hasCurrent ?: state.hasCurrent,
                currentModelName = next?.modelName,
                efforts = next?.efforts ?: state.efforts,
                effort = choice?.effort,
                currentEffort = next?.currentEffort,
                permissionModes = next?.permissionModes ?: state.permissionModes,
                permissionMode = choice?.permissionMode,
                currentPermissionMode = next?.currentPermissionMode,
            )
        }
    }

    fun select(selection: ModelSelection?) = updateTurnChoice { it.copy(model = selection) }
    fun selectEffort(effort: String?) = updateTurnChoice { it.copy(effort = effort) }
    fun selectPermissionMode(modeId: String?) = updateTurnChoice { it.copy(permissionMode = modeId) }
    fun editDraft(text: String) { _state.update { it.copy(draft = text) }; composerChanged() }
    fun removeAttachment(id: String) { _state.update { it.copy(attachments = it.attachments.filter { file -> file.id != id }) }; composerChanged() }

    fun addAttachments(files: List<suspend () -> Pair<String, ByteArray>>) {
        val current = _state.value
        if (files.isEmpty() || current.uploadingAttachment || current.busy || current.inputAccess == InputAccess.READ_ONLY) return
        _state.update { it.copy(uploadingAttachment = true, failure = null) }
        scope.launch {
            try {
                val failures = mutableListOf<String>()
                for (read in files) {
                    try {
                        val (name, bytes) = read()
                        addAttachment(name, bytes)
                    } catch (error: CancellationException) {
                        throw error
                    } catch (error: Exception) {
                        failures += error.describe()
                    }
                }
                if (failures.isNotEmpty()) _state.update { it.copy(failure = failures.distinct().joinToString("\n")) }
            } finally {
                _state.update { it.copy(uploadingAttachment = false) }
            }
        }
    }

    private suspend fun addAttachment(name: String, bytes: ByteArray) {
        val attachment = try {
            io(NativeOperation.CONVERSATION_WRITE) { it.uploadComposerAttachment(name, bytes) }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            error("无法上传“$name”：${error.describe()}")
        }
        _state.update { it.copy(attachments = it.attachments + attachment) }
        composerChanged()
    }
    private fun content(): ComposerInputContent {
        return ComposerInputContent(_state.value.draft, _state.value.attachments)
    }
    private fun cacheComposer() { if (composerCacheBlocked) return; composer?.let { hub.saveComposer(composerKey, SavedComposerInput(it.copy(content = content()), composerPending, composerBaseline, composerUnbound, composerRevision, pendingCreation, composerBindingRevision, composerBindingBaseline, targetChoices, targetRevision, targetPending, targetBaseline)) } }
    private fun composerConversationId(): String? = when (val current = target) {
        is ConversationTarget.Existing -> current.conversationId
        is ConversationTarget.New -> current.draftId?.let { "draft:$it" }
    }

    private fun composerTargetRequest() = ComposerTargetRequest(snapshot?.conversation?.runtimeId ?: target.runtimeId, composerConversationId())

    // Use the draft's launch runtime while retaining the core's choices for each runtime.
    private fun choicesForExecution() = if (target is ConversationTarget.New) {
        targetChoices.copy(runtimeId = null, runtimeOwnerId = null)
    } else targetChoices

    private fun effectiveNextTurn(): NextTurn? = preparedComposerTarget?.let { prepared ->
        if (prepared.request != composerTargetRequest() || snapshot?.let { it.contextRuntimeId != prepared.defaultRuntimeId } == true) return@let null
        resolveComposerTarget(prepared, choicesForExecution(), _state.value.models.takeIf { composerCatalogRuntime == prepared.basis.runtimeId })
    }
    private fun targetContext(owner: String) = composerConversationId()?.let { ComposerContextKey.Conversation(owner, it) } ?: ComposerContextKey.NewConversation
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
            composerSave = null
            try { flushComposer() } catch (error: CancellationException) { throw error } catch (error: Exception) { _state.update { it.copy(composerFailure = composerError(error)) } }
        }
    }
    private suspend fun flushComposer() = composerLock.withLock { saveComposer() }

    private suspend fun saveComposer() {
        if (!composerLoaded) return
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
            val context = targetContext(owner)
            val inputId = composer!!.id
            io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Bind(context, inputId, composerBindingBaseline)) }
            composerUnbound = false; cacheComposer()
        }
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
        val draftContext = creation.draftId?.let { ComposerContextKey.Conversation(creation.ownerId, "draft:$it") }
        if (draftContext != null) io(NativeOperation.CONVERSATION_WRITE) { it.manageComposerInputs(ComposerInputOperation.Forget(draftContext)) }
        pendingCreation = null; cacheComposer()
        val newKey = peer + "/" + (creation.draftId?.let { "draft:$it" } ?: "new")
        if (hub.readComposer(newKey)?.creation?.inputId == creation.inputId) hub.removeComposer(newKey)
        if (oldKey != composerKey && hub.readComposer(oldKey)?.input?.id == creation.inputId) hub.removeComposer(oldKey)
    }
    private fun observeTarget(observed: app.velune.bindings.application.ComposerTarget, owner: String) {
        targetRevision = observed.revision
        val received = normalizeComposerTargetChoices(observed.choices)
        if (targetPending && normalizeComposerTargetChoices(targetChoices) == received) { targetPending = false; targetBaseline = null; targetWriteFailed = false }
        if (!targetPending) {
            targetChoices = received
            val runtime = selectedRuntimeId()
            _state.update {
                it.copy(
                    selectedRuntimeId = runtime,
                    selection = targetChoices.choices.firstOrNull { choice -> choice.runtimeId == runtime && choice.ownerId == owner }?.choice?.model,
                )
            }
        }
    }
    private suspend fun pollComposer(wait: Boolean = false) {
        if (wait) composerLock.lock() else if (!composerLock.tryLock()) return
        val openingRuntime = (target as? ConversationTarget.New)?.runtimeId
        try {
            if (composerCacheBlocked || _state.value.busy || target is ConversationTarget.Existing && snapshot == null) return
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
                val boundInput = directory.inputs.first { it.id == inputId }
                composerBindingRevision = binding.revision
                if (composerUnbound && inputId == composer?.id) { composerUnbound = false; composerBindingBaseline = null }
                else if (composerUnbound && composer?.revision == 0uL) {
                    // A recreated process can still bind a transferred draft over its untouched placeholder.
                    val scopedPlaceholder = (target as? ConversationTarget.New)?.draftId != null &&
                        boundInput.revision == 1uL && boundInput.content == ComposerInputContent("", emptyList())
                    if (!composerRestored || scopedPlaceholder) composerBindingBaseline = binding.revision
                }
                if (!composerPending && !composerUnbound) {
                    composer = boundInput
                    _state.update { it.copy(draft = boundInput.content.text, attachments = boundInput.content.attachments) }
                    cacheComposer()
                }
                directory.targets.firstOrNull { it.context == context }?.let { observeTarget(it, owner) }
            }
            composerLoaded = true
            val overlay = choicesForExecution().takeIf { targetPending || target is ConversationTarget.New }
            val prepared = io(NativeOperation.CONVERSATION_WRITE) { it.prepareComposerTarget(request, overlay) }
            if (request != composerTargetRequest()) return
            check(prepared.ownerId == owner) { "执行端身份已改变，请重新打开会话，输入已保留" }
            preparedComposerTarget = prepared
            observeTarget(prepared.target, owner)
            val choices = choicesForExecution()
            check(choices.runtimeOwnerId == null || choices.runtimeOwnerId == owner) { "所选运行时位于另一设备，输入已保留" }
            val selectedRuntime = choices.runtimeId ?: prepared.defaultRuntimeId
            val catalog = if (composerCatalogRuntime == selectedRuntime) _state.value.models else io(NativeOperation.CONFIGURATION_READ) { it.runtimeModels(selectedRuntime) }
            if (request != composerTargetRequest()) return
            composerCatalogRuntime = selectedRuntime
            val resolved = resolveComposerTarget(prepared, choices, catalog)
            val ready = (snapshot == null || snapshot?.contextRuntimeId == prepared.defaultRuntimeId)
                && resolved?.chosenModelUnavailable == false
            _state.update {
                it.copy(
                    models = resolved?.models ?: catalog,
                    composerReady = ready,
                    hasCurrent = resolved?.hasCurrent ?: it.hasCurrent,
                    currentModelName = resolved?.modelName,
                    efforts = resolved?.efforts ?: emptyList(),
                    currentEffort = resolved?.currentEffort,
                    permissionModes = resolved?.permissionModes ?: emptyList(),
                    currentPermissionMode = resolved?.currentPermissionMode,
                )
            }
            applyResolvedNextTurn()
            saveComposer()
            _state.update {
                it.copy(composerFailure = if (resolved?.chosenModelUnavailable == true) "所选模型在此设备不可用，请重新选择" else null)
            }
        } catch (error: CancellationException) { throw error } catch (error: Exception) { _state.update { it.copy(composerReady = false, composerFailure = composerError(error)) } }
        finally {
            composerLock.unlock()
            val currentRuntime = (target as? ConversationTarget.New)?.runtimeId
            if (currentRuntime != null && currentRuntime != openingRuntime) scope.launch { pollComposer() }
        }
    }

    fun refreshBackgroundTasks() {
        scope.launch { readBackgroundTasks() }
    }

    private fun backgroundTaskTarget(): Pair<String, String>? =
        snapshot?.let { it.contextRuntimeId to it.conversation.id }

    private suspend fun readBackgroundTasks() {
        val (runtime, id) = backgroundTaskTarget() ?: return
        backgroundTaskLock.withLock {
            if (backgroundTaskTarget() != (runtime to id)) return@withLock
            _state.update { it.copy(loadingBackgroundTasks = true, backgroundTaskFailure = null) }
            try {
                val list = io(NativeOperation.CONVERSATION_READ) { it.backgroundTasks(runtime, id) }
                if (backgroundTaskTarget() == (runtime to id)) applyBackgroundTasks(list)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                if (backgroundTaskTarget() == (runtime to id)) {
                    _state.update { it.copy(backgroundTaskFailure = "无法刷新后台任务：${error.describe()}") }
                }
            } finally {
                _state.update { it.copy(loadingBackgroundTasks = false) }
            }
        }
    }

    private fun applyBackgroundTasks(list: BackgroundTaskList) {
        val taskIds = list.tasks.mapTo(mutableSetOf()) { it.id }
        _state.update { state ->
            state.copy(
                backgroundTasks = list,
                backgroundTaskOutputs = state.backgroundTaskOutputs.filterKeys { it in taskIds },
                backgroundTaskOutputFailures = state.backgroundTaskOutputFailures.filterKeys { it in taskIds },
            )
        }
    }

    fun loadBackgroundTaskOutput(taskId: String) {
        val (runtime, id) = backgroundTaskTarget() ?: return
        val current = _state.value
        if (taskId in current.loadingBackgroundTaskOutputs ||
            current.backgroundTasks?.takeIf { it.live }?.tasks?.none { it.id == taskId && it.canReadOutput } != false
        ) return
        val request = current.backgroundTaskOutputs[taskId]?.takeIf { it.status == BackgroundTaskOutputStatus.CHECKING }?.requestId
        _state.update {
            it.copy(
                loadingBackgroundTaskOutputs = it.loadingBackgroundTaskOutputs + taskId,
                backgroundTaskOutputFailures = it.backgroundTaskOutputFailures - taskId,
            )
        }
        scope.launch {
            try {
                val output = io(NativeOperation.CONVERSATION_READ) { it.backgroundTaskOutput(runtime, id, taskId, request) }
                if (backgroundTaskTarget() == (runtime to id)) {
                    _state.update { it.copy(backgroundTaskOutputs = it.backgroundTaskOutputs + (taskId to output)) }
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                if (backgroundTaskTarget() == (runtime to id)) {
                    _state.update {
                        it.copy(backgroundTaskOutputFailures = it.backgroundTaskOutputFailures + (taskId to "无法读取任务输出：${error.describe()}"))
                    }
                }
            } finally {
                _state.update { it.copy(loadingBackgroundTaskOutputs = it.loadingBackgroundTaskOutputs - taskId) }
            }
        }
    }

    fun stopBackgroundTask(taskId: String) {
        val (runtime, id) = backgroundTaskTarget() ?: return
        val current = _state.value
        if (taskId in current.stoppingBackgroundTaskIds ||
            current.backgroundTasks?.takeIf { it.live }?.tasks?.none { it.id == taskId && it.canStop } != false
        ) return
        _state.update { it.copy(stoppingBackgroundTaskIds = it.stoppingBackgroundTaskIds + taskId, backgroundTaskFailure = null) }
        scope.launch {
            try {
                backgroundTaskLock.withLock {
                    if (backgroundTaskTarget() != (runtime to id)) return@withLock
                    val list = io(NativeOperation.CANCEL) { it.stopBackgroundTask(runtime, id, taskId) }
                    if (backgroundTaskTarget() == (runtime to id)) applyBackgroundTasks(list)
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                if (backgroundTaskTarget() == (runtime to id)) {
                    _state.update { it.copy(backgroundTaskFailure = "无法停止后台任务：${error.describe()}") }
                }
            } finally {
                _state.update { it.copy(stoppingBackgroundTaskIds = it.stoppingBackgroundTaskIds - taskId) }
            }
        }
    }

    fun stopSubAgents() = act(onFailure = { failure ->
        _state.update { it.copy(collaborationFailure = "无法停止子 Agent：$failure") }
    }) {
        val current = snapshot ?: return@act
        if (!current.followUp.canStopSubAgents) return@act
        _state.update { it.copy(collaborationFailure = null) }
        apply(io(NativeOperation.CANCEL) { it.stopSubAgents(current.contextRuntimeId, current.conversation.id) })
    }

    suspend fun findLinkedConversation(conversationId: String): ConversationTarget.Existing {
        val conversation = io(NativeOperation.CONVERSATION_READ) { it.findConversation(conversationId) }?.conversation
            ?: error("设备上找不到此会话")
        return ConversationTarget.Existing(conversation.runtimeId, conversation.id, conversation.title.displayText())
    }

    fun send(text: String, onSent: () -> Unit) = send(text, queue = _state.value.running, onSent = onSent)

    fun queue(text: String) = send(text, queue = true, onSent = {})

    private fun send(text: String, queue: Boolean, onSent: () -> Unit) {
        val current = _state.value
        val existing = snapshot
        if (queue && (existing == null || current.running && !current.canQueue)) return
        if (!queue && (current.running || !current.canSend)) return
        submitDraft(text, followUp = "正在提交排队消息…".takeIf { queue }, onSent = onSent) { next, turn, input ->
            val runtime = next.runtimeId
            if (existing != null) {
                if (queue) {
                    val operation = SendQueueOperation.Add(existing.conversation.runtimeId, runtime, next.model, turn)
                    io(NativeOperation.SEND) { it.sendQueue(existing.contextRuntimeId, existing.conversation.id, operation, UUID.randomUUID().toString()) }
                } else {
                    io(NativeOperation.SEND) { it.sendTurn(existing.conversation.runtimeId, existing.conversation.id, runtime, next.model, turn) }
                }
            } else {
                val submittedTarget = choicesForExecution()
                val created = io(NativeOperation.CONVERSATION_WRITE) { it.createConversation(runtime, (target as? ConversationTarget.New)?.cwd, next.model) }
                val oldKey = composerKey
                val snapshot = created.snapshot ?: error("设备没有返回新会话")
                val owner = composerOwner ?: error("输入来源尚未恢复")
                pendingCreation = PendingComposerCreation(input.id, owner, snapshot.conversation.runtimeId, snapshot.conversation.id, submittedTarget, (target as? ConversationTarget.New)?.draftId)
                _state.update { it.copy(composerReady = false) }
                cacheComposer()
                apply(created)
                cacheComposer()
                completeCreation()
                if (hub.readComposer(oldKey)?.input?.id == input.id) hub.removeComposer(oldKey)
                io(NativeOperation.SEND) { it.sendTurn(snapshot.conversation.runtimeId, snapshot.conversation.id, runtime, next.model, turn) }
            }
        }
    }

    fun steer(text: String) {
        val existing = snapshot ?: return
        val current = _state.value
        if (!current.running || !current.canSteer || !current.usesCurrentRuntime || !current.steerUsesCurrentModel) return
        submitDraft(text, followUp = "正在介入本轮…", onSent = {}) { next, turn, _ ->
            // The generated binding steers the current runtime and model.
            val keep = turn.copy(effort = next.choice.effort, permissionMode = next.choice.permissionMode)
            io(NativeOperation.SEND) { it.steerTurn(existing.contextRuntimeId, existing.conversation.id, keep) }
        }
    }

    fun resumeTurn() = act {
        val current = snapshot ?: return@act
        val next = effectiveNextTurn() ?: return@act
        if (!current.followUp.canResume || _state.value.running || !_state.value.composerReady ||
            _state.value.switchingRuntime || next.runtimeId != current.contextRuntimeId
        ) return@act
        apply(io(NativeOperation.SEND) {
            it.resumeTurn(current.contextRuntimeId, current.conversation.id, next.model, next.effort, next.permissionMode?.id)
        })
    }

    private fun submitDraft(
        text: String,
        followUp: String?,
        onSent: () -> Unit,
        submit: suspend (NextTurn, ComposedTurn, ComposerInput) -> SnapshotResult,
    ) {
        val submitted = content()
        val current = _state.value
        if (!current.composerReady || current.composerFailure != null || current.switchingRuntime ||
            current.uploadingAttachment || pendingCreation != null || (text.isBlank() && submitted.attachments.isEmpty())
        ) return
        val submittedGeneration = contentGeneration
        val next = effectiveNextTurn() ?: return
        act(sending = text.takeIf { followUp == null }, submittingFollowUp = followUp) {
            flushComposer()
            val input = composer ?: return@act
            val turn = ComposedTurn(text = text, effort = next.effort, permissionMode = next.permissionMode?.id, attachments = submitted.attachments.map { "composer:" + it.id })
            try {
                apply(submit(next, turn, input))
            } catch (error: BindingException.RemoteRequest) {
                if (error.code != "message_delivery_accepted") throw error
            }
            if (composer?.id == input.id && contentGeneration == submittedGeneration && content() == submitted) {
                _state.update { it.copy(draft = "", attachments = emptyList()) }
                composerChanged()
                flushComposer()
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

    fun setConversationHandoff(release: Boolean) = act {
        val current = snapshot ?: return@act
        val handoff = current.followUp.controlHandoff
        val available = if (release) handoff == ControlHandoff.CAN_RELEASE else handoff == ControlHandoff.RELEASING || handoff == ControlHandoff.RELEASED
        if (!available) return@act
        apply(io(NativeOperation.CONVERSATION_WRITE) { it.setConversationHandoff(current.contextRuntimeId, current.conversation.id, release) })
    }

    fun reply(interactionId: String, reply: RuntimeInteractionReply) = act {
        val current = snapshot ?: return@act
        apply(io(NativeOperation.REPLY) { it.replyRuntimeInteraction(current.contextRuntimeId, current.conversation.id, interactionId, reply) })
    }

    fun editMessage(messageId: String, text: String, placement: EditPlacement, onForked: (ConversationTarget.Existing) -> Unit) = act {
        val current = snapshot ?: return@act
        val editable = _state.value.editable[messageId] ?: return@act
        val original = payloads[messageId] ?: messages[messageId] ?: error("原消息已不可用，请刷新后重试")
        val preservedAttachments = preserveOriginalAttachments(current, original)
        val next = effectiveNextTurn()
        val turn = ComposedTurn(text = text, effort = next?.effort, permissionMode = next?.permissionMode?.id, attachments = preservedAttachments)
        val result = io(NativeOperation.CONVERSATION_WRITE) {
            it.editMessage(current.contextRuntimeId, current.conversation.id, messageId, placement, next?.model, turn, editable.revision)
        }
        if (placement == EditPlacement.FORK) {
            result.snapshot?.let { created ->
                onForked(ConversationTarget.Existing(created.contextRuntimeId, created.conversation.id, created.conversation.title.displayText()))
            }
        } else {
            apply(result)
        }
    }

    /** Re-uploads the original message's image/file attachments so editing text never drops them. */
    private suspend fun preserveOriginalAttachments(current: DeliveredSnapshot, original: DeliveredMessage): List<String> {
        val attachmentIndices = original.blocks.mapIndexedNotNull { index, block ->
            if (block is DeliveredBlock.Image || block is DeliveredBlock.File) index to block else null
        }
        if (attachmentIndices.isEmpty()) return emptyList()
        val needsPayload = attachmentIndices.any { (_, block) ->
            block is DeliveredBlock.Image && block.source is DeliveredImageSource.Data && block.source.data is MessagePayload.Deferred
        }
        val loadedMessage = if (needsPayload) {
            val loaded = io(NativeOperation.CONVERSATION_READ) {
                it.messagePayloads(current.contextRuntimeId, current.conversation.id, listOf(original.id))
            }.firstOrNull { it.id == original.id }
            if (loaded != null && loaded.stamp != original.stamp) error("原消息已有更新，请重新打开编辑")
            loaded
        } else null
        return attachmentIndices.map { (index, block) ->
            val name = attachmentDisplayName(index, block)
            val bytes = when (block) {
                is DeliveredBlock.File -> io(NativeOperation.CONVERSATION_READ) {
                    it.readAttachment(current.contextRuntimeId, current.conversation.id, original.id, index.toUInt()).bytes
                }
                is DeliveredBlock.Image -> when (val source = block.source) {
                    is DeliveredImageSource.Path -> io(NativeOperation.CONVERSATION_READ) {
                        it.readAttachment(current.contextRuntimeId, current.conversation.id, original.id, index.toUInt()).bytes
                    }
                    is DeliveredImageSource.Data -> {
                        val inline = when (val payload = source.data) {
                            is MessagePayload.Inline -> payload.text
                            is MessagePayload.Deferred -> (loadedMessage?.blocks?.getOrNull(index) as? DeliveredBlock.Image)
                                ?.let { (it.source as? DeliveredImageSource.Data)?.data as? MessagePayload.Inline }?.text
                        }
                        if (inline != null) {
                            runCatching { Base64.getMimeDecoder().decode(inline) }.getOrNull()?.takeIf { it.isNotEmpty() }
                                ?: error("无法保留原附件“$name”：原图片数据无法解码")
                        } else {
                            io(NativeOperation.CONVERSATION_READ) {
                                it.readAttachment(current.contextRuntimeId, current.conversation.id, original.id, index.toUInt()).bytes
                            }
                        }
                    }
                }
                else -> error("不支持的原附件类型")
            }
            if (bytes.isEmpty()) error("无法保留原附件“$name”：附件内容为空")
            io(NativeOperation.CONVERSATION_WRITE) { it.uploadAttachment(name, bytes).reference }
        }
    }

    fun forkConversation(messageId: String, onForked: (ConversationTarget.Existing) -> Unit) = act {
        val current = snapshot ?: return@act
        val result = io(NativeOperation.CONVERSATION_WRITE) {
            it.forkConversation(current.contextRuntimeId, current.conversation.id, messageId)
        }
        result.snapshot?.let { created ->
            onForked(ConversationTarget.Existing(created.contextRuntimeId, created.conversation.id, created.conversation.title.displayText()))
        }
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

    private fun act(
        sending: String? = null,
        submittingFollowUp: String? = null,
        onFailure: ((String) -> Unit)? = null,
        body: suspend () -> Unit,
    ) {
        if (_state.value.busy) return
        _state.update { it.copy(busy = true, failure = null, sending = sending, submittingFollowUp = submittingFollowUp) }
        scope.launch {
            try {
                body()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                val failure = error.describe()
                if (onFailure != null) onFailure(failure) else _state.update { it.copy(failure = failure) }
            } finally {
                _state.update { it.copy(busy = false, sending = null, submittingFollowUp = null) }
                if (isActive) pollComposer()
            }
        }
    }
}
