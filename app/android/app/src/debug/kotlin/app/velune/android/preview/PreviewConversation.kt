package app.velune.android.preview

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.velune.android.ConversationActions
import app.velune.android.ConversationContent
import app.velune.android.ConversationHeader
import app.velune.android.ConversationState
import app.velune.android.ConversationTarget
import app.velune.android.NewSessionContext
import app.velune.android.TranscriptEntry
import app.velune.android.canStartConversation
import app.velune.bindings.application.ComposerAttachment
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.application.MessagePayload
import app.velune.bindings.application.TurnChoice
import app.velune.bindings.conversation.BackgroundTaskStatus
import app.velune.bindings.conversation.EditPlacement
import app.velune.bindings.conversation.MessageRole
import app.velune.bindings.conversation.QueuedMessage
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteractionReply
import app.velune.bindings.conversation.ToolState
import app.velune.bindings.conversation.TurnOutcome
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Local state for one open preview, retained while a related screen is on top. */
internal class PreviewConversation(
    initialPeer: String,
    initial: ConversationState,
    initialTarget: ConversationTarget.New? = null,
) {
    private var peer by mutableStateOf(initialPeer)
    private var target by mutableStateOf(initialTarget)
    private val deviceAttachments = mutableMapOf<String, List<ComposerAttachment>>()
    private val choices = mutableMapOf<Pair<String, String>, TurnChoice>()
    private var lastTurnChoice = TurnChoice()
    private var state by mutableStateOf(withRuntime(initial, initialTarget?.runtimeId ?: initial.contextRuntimeId))

    private fun withRuntime(current: ConversationState, runtime: String): ConversationState {
        val catalog = PreviewFixtures.runtimeOptions(runtime)
        val choice = choices[peer to runtime] ?: TurnChoice()
        val sameRuntime = current.conversation != null && current.contextRuntimeId == runtime
        val model = catalog.models.firstOrNull { it.selection == lastTurnChoice.model && sameRuntime } ?: catalog.models.first()
        val efforts = catalog.models.firstOrNull { it.selection == choice.model }?.reasoningEfforts ?: model.reasoningEfforts
        val currentEfforts = model.reasoningEfforts
        val currentPermission = catalog.permissions.firstOrNull { it.id == lastTurnChoice.permissionMode } ?: catalog.permissions.firstOrNull()
        return current.copy(
            canSwitch = current.conversation != null && PreviewFixtures.device(peer).second.runtimes.size > 1,
            selectedRuntimeId = runtime,
            models = catalog.models,
            selection = choice.model,
            hasCurrent = sameRuntime,
            currentModelName = model.name.takeIf { sameRuntime },
            efforts = efforts,
            effort = choice.effort?.takeIf { it in efforts },
            currentEffort = (lastTurnChoice.effort?.takeIf { it in currentEfforts }
                ?: "medium".takeIf { it in currentEfforts } ?: currentEfforts.firstOrNull()).takeIf { sameRuntime },
            permissionModes = catalog.permissions,
            permissionMode = choice.permissionMode,
            currentPermissionMode = currentPermission.takeIf { sameRuntime },
        )
    }

    private fun selectRuntime(runtime: String) {
        if (PreviewFixtures.device(peer).second.runtimes.none { it.id == runtime }) return
        target = target?.copy(runtimeId = runtime)
        state = withRuntime(state, runtime)
    }

    private fun changeChoice(change: (TurnChoice) -> TurnChoice) {
        val runtime = state.selectedRuntimeId
        val key = peer to runtime
        choices[key] = change(choices[key] ?: TurnChoice())
        state = withRuntime(state, runtime)
    }

    private fun selectDevice(next: String) {
        if (next == peer) return
        val (_, listing) = PreviewFixtures.device(next).takeIf { it.canStartConversation() } ?: return
        deviceAttachments[peer] = state.attachments
        peer = next
        val runtime = listing.runtimes.first().id
        target = ConversationTarget.New(runtime, listing.projects.firstOrNull())
        state = withRuntime(state.copy(attachments = deviceAttachments[next].orEmpty()), runtime)
    }

    @Composable
    fun Content(gallery: PreviewGallery) {
        val (device, listing) = PreviewFixtures.device(peer)
        val summary = state.conversation
        val currentTarget = target
        val header = ConversationHeader(
            title = summary?.title?.displayText() ?: "新会话",
            runtime = listing.runtimeLabels[currentTarget?.runtimeId ?: state.contextRuntimeId],
            device = device.name,
            cwd = summary?.cwd ?: currentTarget?.cwd,
        )
        val newSession = currentTarget?.let {
            NewSessionContext(peer, it, PreviewFixtures.listings, ::selectDevice, ::selectRuntime) { cwd ->
                target = target?.copy(cwd = cwd)
            }
        }
        ConversationContent(
            header = header,
            state = state,
            actions = ConversationActions(
                back = gallery::back,
                send = { text, onSent -> submit(text); onSent() },
                stop = ::stop,
                reply = ::answerApproval,
                select = { model -> changeChoice { it.copy(model = model) } },
                load = ::loadTranscriptOutput,
                compact = {
                    val id = previewId()
                    val message = PreviewFixtures.textEntry(id, MessageRole.ASSISTANT, "已压缩会话上下文。")
                    state = state.copy(entries = state.entries + message)
                },
                editDraft = { state = state.copy(draft = it) },
                removeAttachment = { id -> state = state.copy(attachments = state.attachments.filterNot { it.id == id }) },
                addAttachments = { addAttachments(gallery, it) },
                selectEffort = { effort -> changeChoice { it.copy(effort = effort) } },
                selectPermissionMode = { mode -> changeChoice { it.copy(permissionMode = mode) } },
                editMessage = { id, text, placement, _, onDone ->
                    editMessage(gallery, id, text, placement)
                    onDone()
                },
                forkConversation = { forkAt(gallery, it) },
                selectRuntime = ::selectRuntime,
                refresh = {
                    state = state.copy(refreshing = true)
                    gallery.scope.launch {
                        delay(FEEDBACK_DELAY_MS)
                        state = state.copy(refreshing = false)
                    }
                },
                refreshBackgroundTasks = {
                    if (!state.loadingBackgroundTasks) {
                        state = state.copy(loadingBackgroundTasks = true)
                        gallery.scope.launch {
                            delay(FEEDBACK_DELAY_MS)
                            state = state.copy(loadingBackgroundTasks = false)
                        }
                    }
                },
                loadBackgroundTaskOutput = { loadBackgroundTaskOutput(gallery, it) },
                stopBackgroundTask = { stopBackgroundTask(gallery, it) },
                stopSubAgents = { state = state.copy(canStopSubAgents = false) },
                findLinkedConversation = { id ->
                    val linked = checkNotNull(PreviewFixtures.linkedConversation(id, summary?.id)?.conversation)
                    ConversationTarget.Existing(linked.runtimeId, linked.id, linked.title.displayText())
                },
                openConversation = { linked ->
                    PreviewFixtures.linkedConversation(linked.conversationId, summary?.id)?.let {
                        gallery.openConversation(PreviewConversation(peer, it))
                    }
                },
                queue = ::queue,
                steer = ::steer,
                resumeTurn = ::resume,
            ),
            newSession = newSession,
            listing = listing.copy(
                conversations = listing.conversations + PreviewFixtures.linkedListings(state),
                active = listing.active + PreviewFixtures.activeSubAgents(state),
            ),
        )
    }

    private fun submit(text: String) {
        val prompt = submittedText(text)
        val runtime = state.selectedRuntimeId
        val id = previewId()
        val summary = state.conversation ?: PreviewFixtures.summary(
            id, text.lineSequence().firstOrNull()?.take(40)?.ifBlank { null } ?: "附件会话", runtime, target?.cwd,
        )
        lastTurnChoice = TurnChoice(state.selection, state.effort, state.permissionMode)
        target = null
        state = withRuntime(state.copy(
            conversation = summary,
            contextRuntimeId = runtime,
            runState = RunState.IDLE,
            canResume = false,
            canFork = true,
            entries = state.entries + PreviewFixtures.completedTurn(id, prompt, "已收到消息，示例执行已完成。"),
            editable = state.editable + ("user:$id" to PreviewFixtures.editable("user:$id")),
            draft = "",
            attachments = emptyList(),
        ), runtime)
    }

    private fun queue(text: String) {
        val attachments = state.attachments.map { "/preview/attachments/${it.name}" }
        state = state.copy(
            queue = state.queue + QueuedMessage(previewId(), text = text, attachments = attachments),
            draft = "",
            attachments = emptyList(),
        )
    }

    private fun steer(text: String) {
        val message = PreviewFixtures.textEntry(previewId(), MessageRole.USER, submittedText(text))
        val work = state.entries.lastOrNull() as? TranscriptEntry.Work
        val entries = if (work?.running == true) state.entries.dropLast(1) + message + work else state.entries + message
        state = state.copy(entries = entries, draft = "", attachments = emptyList())
    }

    private fun submittedText(text: String) = (listOf(text) + state.attachments.map { "附件：${it.name}" })
        .filter { it.isNotBlank() }.joinToString("\n")

    private fun answerApproval(id: String, reply: RuntimeInteractionReply) {
        state = state.copy(interactions = state.interactions.filterNot { it.id == id })
        if (reply is RuntimeInteractionReply.Decision && reply.optionId == "deny") {
            finishWork(RunState.FAILED, TurnOutcome.FAILED, "命令未获批准。")
        }
    }

    private fun stop() = finishWork(RunState.IDLE, TurnOutcome.CANCELLED, "示例执行已停止。")

    private fun finishWork(runState: RunState, outcome: TurnOutcome, detail: String) {
        state = state.copy(
            runState = runState,
            canCancel = false,
            canResume = outcome == TurnOutcome.FAILED,
            interactions = emptyList(),
            entries = state.entries.map { entry ->
                if (entry is TranscriptEntry.Work && entry.running) {
                    entry.withToolState(ToolState.FAILED, MessagePayload.Inline(detail)).copy(running = false, outcome = outcome)
                } else entry
            },
        )
    }

    private fun resume() {
        val previous = state.entries.lastOrNull() as? TranscriptEntry.Work ?: return
        state = state.copy(
            runState = RunState.RUNNING,
            canCancel = true,
            canResume = false,
            canQueue = true,
            canSteer = true,
            entries = state.entries.dropLast(1) + previous.withToolState(ToolState.RUNNING, null)
                .copy(running = true, durationMs = null, outcome = null),
        )
    }

    private fun TranscriptEntry.Work.withToolState(status: ToolState, output: MessagePayload?) = copy(
        members = members.map { message ->
            message.copy(blocks = message.blocks.map { block ->
                if (block is DeliveredBlock.Tool) block.copy(state = status, output = output) else block
            })
        },
    )

    private fun editMessage(gallery: PreviewGallery, id: String, text: String, placement: EditPlacement) {
        val index = messageIndex(id)
        if (index < 0) return
        val prior = state.entries.take(index)
        if (placement == EditPlacement.FORK) {
            val fork = fork(prior)
            fork.submit(text)
            gallery.openConversation(fork)
        } else {
            val retainedIds = prior.filterIsInstance<TranscriptEntry.Message>().map { it.message.id }.toSet()
            state = state.copy(entries = prior, editable = state.editable.filterKeys { it in retainedIds },
                interactions = emptyList(), canCancel = false)
            val editId = previewId()
            state = state.copy(
                entries = state.entries + PreviewFixtures.completedTurn(editId, text, "已按修改后的消息重新执行。"),
                editable = state.editable + ("user:$editId" to PreviewFixtures.editable("user:$editId")),
                runState = RunState.IDLE,
                canResume = false,
            )
        }
    }

    private fun forkAt(gallery: PreviewGallery, id: String) {
        val index = messageIndex(id)
        if (index >= 0) gallery.openConversation(fork(state.entries.take(index + 1)))
    }

    private fun messageIndex(id: String) = state.entries.indexOfFirst { it is TranscriptEntry.Message && it.message.id == id }

    private fun fork(entries: List<TranscriptEntry>): PreviewConversation {
        val original = checkNotNull(state.conversation)
        val retainedIds = entries.filterIsInstance<TranscriptEntry.Message>().map { it.message.id }.toSet()
        return PreviewConversation(peer, ConversationState(
            conversation = PreviewFixtures.summary(previewId(), "${original.title.displayText()}（分叉）", state.contextRuntimeId, original.cwd),
            contextRuntimeId = state.contextRuntimeId,
            entries = entries,
            editable = state.editable.filterKeys { it in retainedIds },
            canFork = true,
            composerReady = true,
            loading = false,
        ))
    }

    private fun loadTranscriptOutput(id: String) {
        fun load(message: DeliveredMessage) = if (message.id == id) {
            message.copy(blocks = message.blocks.map { block ->
                if (block is DeliveredBlock.Tool && block.output is MessagePayload.Deferred) {
                    block.copy(output = PreviewFixtures.deferredBuildOutput())
                } else block
            })
        } else message
        state = state.copy(entries = state.entries.map { entry ->
            when (entry) {
                is TranscriptEntry.Message -> entry.copy(message = load(entry.message))
                is TranscriptEntry.Work -> entry.copy(members = entry.members.map(::load))
            }
        })
    }

    private fun addAttachments(gallery: PreviewGallery, files: List<suspend () -> Pair<String, ByteArray>>) {
        if (files.isEmpty() || state.uploadingAttachment) return
        state = state.copy(uploadingAttachment = true, failure = null)
        gallery.scope.launch {
            try {
                for (read in files) {
                    try {
                        val (name, bytes) = read()
                        val file = ComposerAttachment(previewId(), name, bytes.size.toULong())
                        state = state.copy(attachments = state.attachments + file)
                    } catch (error: CancellationException) {
                        throw error
                    } catch (error: Exception) {
                        state = state.copy(failure = error.message ?: "无法读取附件")
                    }
                }
            } finally {
                state = state.copy(uploadingAttachment = false)
            }
        }
    }

    private fun loadBackgroundTaskOutput(gallery: PreviewGallery, id: String) {
        if (id in state.loadingBackgroundTaskOutputs || id in state.backgroundTaskOutputs) return
        state = state.copy(loadingBackgroundTaskOutputs = state.loadingBackgroundTaskOutputs + id)
        gallery.scope.launch {
            delay(FEEDBACK_DELAY_MS)
            state = state.copy(
                loadingBackgroundTaskOutputs = state.loadingBackgroundTaskOutputs - id,
                backgroundTaskOutputs = state.backgroundTaskOutputs + (id to PreviewFixtures.backgroundOutput(id)),
            )
        }
    }

    private fun stopBackgroundTask(gallery: PreviewGallery, id: String) {
        if (id in state.stoppingBackgroundTaskIds) return
        state = state.copy(stoppingBackgroundTaskIds = state.stoppingBackgroundTaskIds + id)
        gallery.scope.launch {
            delay(FEEDBACK_DELAY_MS)
            val tasks = state.backgroundTasks
            state = state.copy(
                stoppingBackgroundTaskIds = state.stoppingBackgroundTaskIds - id,
                backgroundTasks = tasks?.copy(tasks = tasks.tasks.map { task ->
                    if (task.id == id) task.copy(status = BackgroundTaskStatus.CANCELLED, canStop = false) else task
                }),
            )
        }
    }

    companion object {
        private const val FEEDBACK_DELAY_MS = 450L
        private fun previewId() = UUID.randomUUID().toString()

        fun newSession(preferredPeer: String?): PreviewConversation {
            val available = PreviewFixtures.listings.filter { it.canStartConversation() }
            val (device, listing) = available.firstOrNull { it.first.peerId == preferredPeer } ?: available.first()
            return PreviewConversation(
                device.peerId,
                PreviewFixtures.newConversation(),
                ConversationTarget.New(listing.runtimes.first().id, listing.projects.firstOrNull()),
            )
        }
    }
}
