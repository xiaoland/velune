package app.velune.android.preview

import app.velune.android.ConversationState
import app.velune.android.DeviceListing
import app.velune.android.DeviceStatus
import app.velune.android.DeviceSummary
import app.velune.android.RuntimeLabel
import app.velune.android.Tone
import app.velune.android.TranscriptEntry
import app.velune.bindings.application.ActiveConversation
import app.velune.bindings.application.ComposerAttachment
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.application.ListedConversation
import app.velune.bindings.application.MessagePayload
import app.velune.bindings.application.PermissionMode
import app.velune.bindings.application.RuntimeInstance
import app.velune.bindings.application.RuntimeModel
import app.velune.bindings.conversation.BackgroundTaskInfo
import app.velune.bindings.conversation.BackgroundTaskList
import app.velune.bindings.conversation.BackgroundTaskOutput
import app.velune.bindings.conversation.BackgroundTaskOutputStatus
import app.velune.bindings.conversation.BackgroundTaskStatus
import app.velune.bindings.conversation.Collaborator
import app.velune.bindings.conversation.ConversationSummary
import app.velune.bindings.conversation.ConversationTitle
import app.velune.bindings.conversation.EditableMessage
import app.velune.bindings.conversation.InteractionKind
import app.velune.bindings.conversation.InteractionOption
import app.velune.bindings.conversation.MessageRole
import app.velune.bindings.conversation.ModelSelection
import app.velune.bindings.conversation.QueuedMessage
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteraction
import app.velune.bindings.conversation.RuntimeKind
import app.velune.bindings.conversation.SubAgent
import app.velune.bindings.conversation.ToolState
import app.velune.bindings.conversation.TurnOutcome

internal data class PreviewRuntimeOptions(val models: List<RuntimeModel>, val permissions: List<PermissionMode>)

/** Synthetic data shared by the debug scenarios. Each conversation factory creates a fresh starting state. */
internal object PreviewFixtures {
    const val macPeer = "mac"
    const val windowsPeer = "win"
    private const val macProject = "/Users/demo/Development/velune"
    private const val windowsProject = "C:\\work\\velune"
    private const val completedId = "c2"
    private const val runningId = "c1"
    private const val failedId = "c4"
    private const val parentId = "claude:release"
    private const val childId = "claude:list-review"
    private const val collaboratorId = "preview-a2a-build-review"
    private val now = System.currentTimeMillis()

    // Runtime, model, reasoning and permission catalogs.
    private val runtimes = listOf(
        RuntimeInstance(true, "claude", "Claude Code", RuntimeKind.CLAUDE_CODE, "gateway", emptyMap()),
        RuntimeInstance(true, "codex", "Codex", RuntimeKind.CODEX166, "gateway", emptyMap()),
        RuntimeInstance(true, "pi", "Pi", RuntimeKind.PI, "gateway", emptyMap()),
    )
    private val runtimeLabels = mapOf(
        "claude" to RuntimeLabel("Claude Code", "claude-code"),
        "codex" to RuntimeLabel("Codex", "codex"),
        "pi" to RuntimeLabel("Pi", "pi"),
    )
    private val runtimeOptions = mapOf(
        "claude" to PreviewRuntimeOptions(
            listOf(
                model("claude-opus-preview", "Claude Opus", "low", "medium", "high"),
                model("claude-sonnet-preview", "Claude Sonnet", "low", "medium"),
            ),
            listOf(
                PermissionMode("default", "默认审批", "执行命令或修改敏感内容前询问"),
                PermissionMode("acceptEdits", "自动接受编辑", "自动写入文件，终端命令仍需确认"),
                PermissionMode("plan", "仅规划", "只读分析与制定方案"),
            ),
        ),
        "codex" to PreviewRuntimeOptions(
            listOf(
                model("gpt-codex-preview", "GPT Codex", "low", "medium", "high", "xhigh"),
                model("gpt-codex-mini-preview", "GPT Codex Mini", "low", "medium"),
            ),
            listOf(
                PermissionMode("workspace", "工作区写入", "在项目目录内编辑，其他操作需要审批"),
                PermissionMode("read-only", "只读", "只允许读取工作区"),
            ),
        ),
        "pi" to PreviewRuntimeOptions(
            listOf(model("pi-preview", "Pi 示例模型", "low", "high"), model("pi-fast-preview", "Pi 快速模型")),
            listOf(PermissionMode("project", "项目内执行", "允许在当前项目目录执行")),
        ),
    )

    fun runtimeOptions(runtime: String) = runtimeOptions.getValue(runtime)

    private fun model(id: String, name: String, vararg efforts: String) =
        RuntimeModel(ModelSelection.Native(null, id), name, efforts.toList())

    // Devices and the aggregate session list.
    val devices = listOf(
        DeviceSummary(macPeer, "工作室 Mac mini", DeviceStatus("已连接 · 直连 12 ms", Tone.Positive), true),
        DeviceSummary(windowsPeer, "win-ws", DeviceStatus("已连接 · 中继 48 ms", Tone.Positive), true),
        DeviceSummary("laptop", "MacBook Air", DeviceStatus("未连接", Tone.Neutral), true),
    )

    private val completedSummary = summary(completedId, "修复 Android 会话列表的滚动抖动", "claude", macProject, minutesAgo = 2)
    private val runningSummary = summary(runningId, "发布前检查清单", "claude", macProject, minutesAgo = 1)
    private val failedSummary = summary(failedId, "排查 Windows 子进程控制台窗口", "codex", windowsProject, minutesAgo = 95)
    private val parentSummary = summary(parentId, "Android 发布准备", "claude", macProject)
    private val childSummary = summary(childId, "检查 Compose 列表键", "claude", macProject, parent = completedId)
    private val collaboratorSummary = summary(collaboratorId, "核对构建产物", "codex", macProject)

    val listings = listOf(
        devices[0] to DeviceListing(
            conversations = listOf(
                listed(runningSummary, pinned = true),
                listed(completedSummary),
                listed(summary("c3", "为 remote 包补充断线重连文档", "codex", macProject, 18), unread = true),
                listed(summary("c5", "整理博客草稿的图片", "codex", "/Users/demo/Writing/blog", 60 * 26)),
                listed(summary("c6", "解释这段正则表达式", "pi", null, 60 * 72)),
            ),
            runtimes = runtimes,
            runtimeLabels = runtimeLabels,
            projectNames = mapOf(macProject to "velune", "/Users/demo/Writing/blog" to "博客"),
            projects = listOf(macProject, "/Users/demo/Writing/blog", "/Users/demo/Development/design-system"),
            active = mapOf(runningId to ActiveConversation("claude", runningId, RunState.RUNNING, 1u, null)),
            loaded = true,
            path = "直连 12 ms",
        ),
        devices[1] to DeviceListing(
            conversations = listOf(listed(failedSummary, unread = true, failed = true)),
            runtimes = runtimes.take(2),
            runtimeLabels = runtimeLabels,
            projectNames = mapOf(windowsProject to "velune"),
            projects = listOf(windowsProject, "C:\\work\\release-tools"),
            loaded = true,
            path = "中继 48 ms",
        ),
        devices[2] to DeviceListing(loaded = true, failure = "设备未在线"),
    )
    val emptyListings = listOf(devices[0] to listings[0].second.copy(conversations = emptyList(), active = emptyMap()))

    fun device(peer: String) = listings.first { it.first.peerId == peer }

    private fun listed(summary: ConversationSummary, pinned: Boolean = false, unread: Boolean = false, failed: Boolean = false) =
        ListedConversation(summary, summary.id, null, pinned, unread, false, summary.cwd, false, failed, null)

    fun summary(id: String, title: String, runtime: String, cwd: String?, minutesAgo: Int = 0, parent: String? = null) =
        ConversationSummary(id, ConversationTitle.Native(title), now - minutesAgo * 60_000L, now - minutesAgo * 60_000L,
            runtime, cwd, canRename = true, canDelete = true, parentConversationId = parent, nativeReadState = null)

    // Completed, running and recoverable sessions.
    fun completedConversation(): ConversationState = existing(completedSummary).copy(
        entries = completedTurn(
            "list",
            "会话列表滚动时会抖一下，帮我看看为什么，顺便修掉。",
            """
                找到原因了：列表每次轮询都换了 **key**，导致行被重新创建。

                1. 用运行时和会话 ID 组成稳定的 key
                2. 只在快照的 cursor 变化时更新列表

                ```kotlin
                items(conversations, key = { it.runtimeId + "/" + it.id }) { row ->
                    ConversationRow(row)
                }
                ```

                | 指标 | 修改前 | 修改后 |
                | --- | --- | --- |
                | 每次轮询重组 | 42 | 3 |
                | 掉帧 | 明显 | 无 |
            """.trimIndent(),
            work = work("list", blocks = listOf(
                DeliveredBlock.Reasoning("先看列表的 key 与轮询频率", MessagePayload.Inline("如果 key 不稳定，Compose 会把每一行当作新行。")),
                tool("read", "Read SessionsScreen.kt", ToolState.COMPLETED, MessagePayload.Inline("import androidx.compose.foundation.lazy.LazyColumn")),
                tool("build", "Bash ./gradlew :app:assembleDebug", ToolState.FAILED, MessagePayload.Deferred(48_213uL)),
                tool("edit", "Edit SessionsScreen.kt", ToolState.COMPLETED, MessagePayload.Inline("+ items(conversations, key = { it.runtimeId + \"/\" + it.id })")),
                tool("retry", "Bash ./gradlew :app:assembleDebug", ToolState.COMPLETED, MessagePayload.Inline("BUILD SUCCESSFUL")),
            )),
        ),
        editable = mapOf("user:list" to editable("user:list")),
        canCompact = true,
        attachments = attachmentSamples(),
        backgroundTasks = completedTasks(),
        parentConversationId = parentId,
        subAgents = subAgents(),
        collaborators = collaborators(),
    )

    fun runningConversation(): ConversationState = existing(runningSummary).copy(
        entries = completedTurn("release-plan", "先列一下 Android 发版前要做的检查。", "需要检查 debug 构建、真机安装和会话列表。") +
            textEntry("user:release-run", MessageRole.USER, "顺便跑一下 Android 构建。") +
            work("release-run", running = true, blocks = listOf(
                tool("release-build", "Bash ./gradlew :app:assembleDebug", ToolState.RUNNING),
            )),
        runState = RunState.RUNNING,
        canCancel = true,
        canQueue = true,
        canSteer = true,
        interactions = listOf(approval()),
        backgroundTasks = runningTasks(),
        subAgents = subAgents(),
        canStopSubAgents = true,
        collaborators = collaborators(),
        queue = queuedMessages(),
    )

    fun failedConversation(): ConversationState = existing(failedSummary).copy(
        entries = listOf(
            textEntry("user:windows", MessageRole.USER, "检查启动子进程时为什么弹出了控制台窗口。"),
            work("windows", outcome = TurnOutcome.FAILED, blocks = listOf(
                tool("windows-test", "PowerShell .\\check-process.ps1", ToolState.FAILED,
                    MessagePayload.Inline("子进程启动检查超时，可从检查步骤继续。")),
            )),
        ),
        runState = RunState.FAILED,
        canResume = true,
    )

    fun newConversation() = ConversationState(loading = false, composerReady = true)

    fun listedConversation(peer: String, id: String): ConversationState = when (id) {
        completedId -> completedConversation()
        runningId -> runningConversation()
        failedId -> failedConversation()
        else -> {
            val item = device(peer).second.conversations.first { it.conversation.id == id }
            val summary = item.conversation
            briefConversation(summary, summary.title.displayText())
        }
    }

    private fun existing(summary: ConversationSummary) = ConversationState(
        conversation = summary,
        loading = false,
        canFork = true,
        contextRuntimeId = summary.runtimeId,
        composerReady = true,
    )

    private fun approval() = RuntimeInteraction(
        "release-build-approval",
        InteractionKind.Approval(
            "允许运行命令？",
            "./gradlew :app:assembleDebug",
            listOf(InteractionOption("allow", "允许"), InteractionOption("always", "本会话都允许"), InteractionOption("deny", "拒绝")),
        ),
    )

    // Background tasks, their on-demand output, and collaboration.
    private fun completedTasks() = BackgroundTaskList(
        listOf(BackgroundTaskInfo("android-tests", "command", "会话列表回归检查", BackgroundTaskStatus.COMPLETED,
            completedId, null, canStop = false, canReadOutput = true)),
        live = true,
        issue = null,
    )

    private fun runningTasks() = BackgroundTaskList(
        listOf(
            BackgroundTaskInfo("debug-build", "command", "Android Debug 构建", BackgroundTaskStatus.RUNNING,
                runningId, null, canStop = true, canReadOutput = true),
            BackgroundTaskInfo("release-lint", "command", "检查发布清单", BackgroundTaskStatus.COMPLETED,
                runningId, null, canStop = false, canReadOutput = true),
        ),
        live = true,
        issue = null,
    )

    fun backgroundOutput(taskId: String) = BackgroundTaskOutput(
        "preview-output/$taskId", taskId, BackgroundTaskOutputStatus.READY,
        text = when (taskId) {
            "android-tests" -> "SessionListTest > stableItemKeys PASSED\nSessionListTest > deviceFilter PASSED"
            "debug-build" -> "> Task :app:compileDebugKotlin\n> Task :app:mergeDebugResources"
            "release-lint" -> "发布清单检查完成\n4 项通过"
            else -> error("缺少后台任务输出样本：$taskId")
        },
        truncated = false,
        issue = null,
    )

    private fun subAgents() = listOf(SubAgent(childId, "检查 Compose 列表键", failed = false))
    private fun collaborators() = listOf(Collaborator(collaboratorId, "核对构建产物", "Codex", contacted = true, contactedBy = true))

    fun linkedListings(state: ConversationState): List<ListedConversation> = listOfNotNull(
        parentSummary.takeIf { state.parentConversationId == it.id }?.let { listed(it) },
        childSummary.takeIf { child -> state.subAgents.any { it.conversationId == child.id } }?.let { child ->
            listed(child.copy(parentConversationId = state.conversation?.id))
        },
    )

    fun linkedConversation(id: String, sourceId: String?): ConversationState? = when (id) {
        completedId -> completedConversation()
        runningId -> runningConversation()
        parentId -> briefConversation(parentSummary, "汇总 Android 发布准备事项。")
        childId -> briefConversation(childSummary.copy(parentConversationId = sourceId), "检查会话列表是否使用稳定的 key。")
            .copy(parentConversationId = sourceId)
        collaboratorId -> briefConversation(collaboratorSummary, "核对 Debug APK 和构建产物清单。")
        else -> null
    }

    fun activeSubAgents(state: ConversationState) = if (state.canStopSubAgents) {
        state.subAgents.associate { it.conversationId to ActiveConversation("claude", it.conversationId, RunState.RUNNING, 0u, null) }
    } else emptyMap()

    private fun briefConversation(summary: ConversationSummary, prompt: String) = existing(summary).copy(
        entries = completedTurn(summary.id, prompt, "检查完成，结果已汇总到对应的会话。"),
    )

    // Composer and transcript samples shared with the simulated actions.
    private fun attachmentSamples() = listOf(
        ComposerAttachment("preview-image", "scroll-trace.png", 18_420uL),
        ComposerAttachment("preview-log", "compose-recomposition.txt", 2_140uL),
    )

    private fun queuedMessages() = listOf(
        QueuedMessage("preview-queue", text = "构建结束后整理检查结果。", attachments = listOf("$macProject/release-checklist.md")),
    )

    fun editable(id: String) = EditableMessage(id, inPlace = true, deleteOnly = false, deleteFollowing = false, revision = "preview/$id")

    fun deferredBuildOutput() = MessagePayload.Inline("构建首次失败：无法解析列表项的 key。修正后重试通过。")

    fun textEntry(id: String, role: MessageRole, text: String, canFork: Boolean = false) =
        TranscriptEntry.Message(id, message(id, role, listOf(DeliveredBlock.Text(text))), canFork)

    fun completedTurn(id: String, prompt: String, reply: String, work: TranscriptEntry.Work? = null) = listOfNotNull(
        textEntry("user:$id", MessageRole.USER, prompt),
        work,
        textEntry("reply:$id", MessageRole.ASSISTANT, reply, canFork = true),
    )

    private fun work(id: String, running: Boolean = false, outcome: TurnOutcome? = null, blocks: List<DeliveredBlock>) =
        TranscriptEntry.Work("work:$id", listOf(message("work-message:$id", MessageRole.ASSISTANT, blocks)),
            durationMs = 83_000uL.takeUnless { running }, running = running, outcome = outcome ?: TurnOutcome.COMPLETED.takeUnless { running })

    private fun message(id: String, role: MessageRole, blocks: List<DeliveredBlock>) =
        DeliveredMessage(1uL, true, id, role, now, blocks)

    private fun tool(id: String, title: String, state: ToolState, output: MessagePayload? = null) =
        DeliveredBlock.Tool(id, title, state, output, null)
}
