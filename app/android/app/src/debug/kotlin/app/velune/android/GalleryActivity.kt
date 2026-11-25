package app.velune.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.velune.bindings.application.ActiveConversation
import app.velune.bindings.application.DeliveredBlock
import app.velune.bindings.application.DeliveredMessage
import app.velune.bindings.application.ListedConversation
import app.velune.bindings.application.MessagePayload
import app.velune.bindings.application.PermissionMode
import app.velune.bindings.application.RuntimeInstance
import app.velune.bindings.application.RuntimeModel
import app.velune.bindings.conversation.ConversationSummary
import app.velune.bindings.conversation.ConversationTitle
import app.velune.bindings.conversation.EditableMessage
import app.velune.bindings.conversation.InteractionKind
import app.velune.bindings.conversation.InteractionOption
import app.velune.bindings.conversation.MessageRole
import app.velune.bindings.conversation.ModelSelection
import app.velune.bindings.conversation.RunState
import app.velune.bindings.conversation.RuntimeInteraction
import app.velune.bindings.conversation.RuntimeKind
import app.velune.bindings.conversation.ToolState
import app.velune.bindings.conversation.TranscriptItem
import app.velune.bindings.conversation.TranscriptTurn
import app.velune.bindings.conversation.TurnOutcome

/**
 * Debug builds only: each screen's content with made-up data, for looking at
 * layouts on a phone without a paired device or real conversations.
 *
 *     adb shell am start -n app.velune.android/.GalleryActivity --es screen sessions
 *
 * Screens: sessions, empty-sessions, no-devices, devices, conversation, running, new.
 */
class GalleryActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val screen = intent.getStringExtra("screen") ?: "sessions"
        setContent {
            VeluneTheme {
                Surface(Modifier.fillMaxSize()) {
                    when (screen) {
                        "devices" -> DevicesContent(Sample.devices, null, DevicesActions({}, {}, {}, {}))
                        "no-devices" -> SessionsContent(emptyList(), emptyList(), null, null, false, Sample.sessionsActions)
                        "empty-sessions" -> SessionsContent(Sample.devices.take(1), listOf(Sample.devices[0] to Sample.macListing.copy(conversations = emptyList())), null, null, false, Sample.sessionsActions)
                        "sessions", "list" -> {
                            var filter by remember { mutableStateOf<String?>(null) }
                            SessionsContent(Sample.devices, Sample.listings, filter, null, false, SessionsActions({ _, _ -> }, {}, { filter = it }, {}, {}, {}, {}))
                        }
                        "running" -> InteractiveConversation(Sample.header, Sample.running)
                        "new" -> InteractiveConversation(Sample.header.copy(title = "新会话"), ConversationState(loading = false, models = Sample.models, efforts = Sample.efforts, permissionModes = Sample.permissionModes, composerReady = true))
                        else -> InteractiveConversation(Sample.header, Sample.finished)
                    }
                }
            }
        }
    }
}

@Composable
private fun InteractiveConversation(header: ConversationHeader, initial: ConversationState) {
    var state by remember { mutableStateOf(initial) }
    ConversationContent(
        header,
        state,
        ConversationActions(
            back = {},
            send = { _, _ -> state = state.copy(draft = "") },
            stop = {},
            reply = { _, _ -> state = state.copy(interactions = emptyList()) },
            select = { state = state.copy(selection = it) },
            load = {},
            compact = {},
            editDraft = { state = state.copy(draft = it) },
            removeAttachment = {},
            setConversationHandoff = {},
            selectEffort = { state = state.copy(effort = it) },
            selectPermissionMode = { state = state.copy(permissionMode = it) },
            editMessage = { _, _, _ -> },
            forkConversation = {},
        ),
    )
}

private object Sample {
    private val now = System.currentTimeMillis()
    private const val MINUTE = 60_000L

    val sessionsActions = SessionsActions({ _, _ -> }, {}, {}, {}, {}, {}, {})

    val devices = listOf(
        DeviceSummary("mac", "工作室 Mac mini", DeviceStatus("已连接 · 直连 12 ms", Tone.Positive), true),
        DeviceSummary("win", "win-ws", DeviceStatus("已连接 · 中继 48 ms", Tone.Positive), true),
        DeviceSummary("laptop", "MacBook Air", DeviceStatus("未连接", Tone.Neutral), true),
    )

    private val runtimes = listOf(
        RuntimeInstance(true, "claude", "Claude Code", RuntimeKind.CLAUDE_CODE, "gateway", emptyMap()),
        RuntimeInstance(true, "codex", "Codex", RuntimeKind.CODEX166, "gateway", emptyMap()),
        RuntimeInstance(true, "pi", "Pi", RuntimeKind.PI, "gateway", emptyMap()),
    )

    private val labels = mapOf(
        "claude" to RuntimeLabel("Claude Code", "claude-code"),
        "codex" to RuntimeLabel("Codex", "codex"),
        "pi" to RuntimeLabel("Pi", "pi"),
    )

    private fun listed(id: String, title: String, runtime: String, minutesAgo: Long, project: String?, pinned: Boolean = false, unread: Boolean = false, failed: Boolean = false) =
        ListedConversation(
            ConversationSummary(id, ConversationTitle.Native(title), now - minutesAgo * MINUTE, now - minutesAgo * MINUTE, runtime, project, true, true, null, null),
            id, "agents://$runtime/11111111-1111-4111-8111-111111111111", pinned, unread, false, project, false, failed, null,
        )

    val macListing = DeviceListing(
        conversations = listOf(
            listed("c1", "发布前检查清单", "claude", 2, "/Users/me/Development/velune", pinned = true),
            listed("c2", "修复 Android 会话列表的滚动抖动", "claude", 1, "/Users/me/Development/velune"),
            listed("c3", "为 remote 包补充断线重连文档", "codex", 18, "/Users/me/Development/velune", unread = true),
            listed("c5", "整理博客草稿的图片", "codex", 60 * 26, "/Users/me/Writing/blog"),
            listed("c6", "解释这段正则表达式", "pi", 60 * 72, null),
        ),
        runtimes = runtimes,
        runtimeLabels = labels,
        projectNames = mapOf("/Users/me/Development/velune" to "velune"),
        projects = listOf("/Users/me/Development/velune", "/Users/me/Writing/blog"),
        active = mapOf(
            "c2" to ActiveConversation("claude", "c2", RunState.RUNNING, 0u, null),
            "c1" to ActiveConversation("claude", "c1", RunState.RUNNING, 1u, null),
        ),
        loaded = true,
        path = "直连 12 ms",
    )

    private val winListing = DeviceListing(
        conversations = listOf(
            listed("c4", "排查 Windows 子进程控制台窗口", "pi", 95, "C:\\work\\velune", failed = true),
        ),
        runtimes = runtimes.take(2),
        runtimeLabels = labels,
        projectNames = mapOf("C:\\work\\velune" to "velune"),
        projects = listOf("C:\\work\\velune"),
        active = emptyMap(),
        loaded = true,
        path = "中继 48 ms",
    )

    val listings = listOf(
        devices[0] to macListing,
        devices[1] to winListing,
        devices[2] to DeviceListing(loaded = true, failure = "设备未在线"),
    )

    val models = listOf(
        RuntimeModel(ModelSelection.Native(null, "claude-opus-5-5"), "Claude Opus 5.5", listOf("low", "medium", "high")),
        RuntimeModel(ModelSelection.Native(null, "claude-sonnet-5-5"), "Claude Sonnet 5.5", listOf("low", "medium", "high")),
    )

    val efforts = listOf("low", "medium", "high")

    val permissionModes = listOf(
        PermissionMode("default", "默认审批", "执行命令或修改敏感内容前询问"),
        PermissionMode("acceptEdits", "自动接受编辑", "自动写入文件，终端命令仍需确认"),
        PermissionMode("plan", "仅规划", "只读分析与制定方案，不改动文件"),
    )

    val header = ConversationHeader("修复 Android 会话列表的滚动抖动", RuntimeLabel("Claude Code", "claude-code"), "工作室 Mac mini", "/Users/me/Development/velune")

    private fun message(id: String, role: MessageRole, minutesAgo: Long, vararg blocks: DeliveredBlock) =
        DeliveredMessage(1uL, true, id, role, now - minutesAgo * MINUTE, blocks.toList())

    private val reply = """
        找到原因了：`LazyColumn` 每次轮询都换了 **key**，所以整页重新测量。

        修改分三步：

        1. 用会话的 `runtimeId/id` 作为稳定 key
        2. 只在快照的 `cursor` 变化时更新列表
        3. 把相对时间的计算移出组合

        ```kotlin
        items(conversations, key = { it.runtimeId + "/" + it.id }) { row ->
            ConversationRow(row)
        }
        ```

        | 指标 | 修改前 | 修改后 |
        | --- | --- | --- |
        | 每次轮询重组 | 42 | 3 |
        | 掉帧 | 明显 | 无 |

        详见 [Compose 列表文档](https://developer.android.com/develop/ui/compose/lists)。
    """.trimIndent()

    private val messages = listOf(
        message("u1", MessageRole.USER, 5, DeliveredBlock.Text("会话列表滚动时会抖一下，帮我看看为什么，顺便修掉。")),
        message(
            "a1", MessageRole.ASSISTANT, 4,
            DeliveredBlock.Reasoning("先看列表的 key 与轮询频率", MessagePayload.Inline("列表每两秒轮询一次；如果 key 不稳定，Compose 会把每一行当作新行。")),
            DeliveredBlock.Tool("t1", "Read app/android/…/SessionsScreen.kt", ToolState.COMPLETED, MessagePayload.Inline("  1 package app.velune.android\n  2 \n  3 import androidx.compose.foundation.lazy.LazyColumn\n…"), null),
            DeliveredBlock.Tool("t2", "Bash ./gradlew :app:assembleDebug", ToolState.FAILED, MessagePayload.Deferred(48_213uL), null),
            DeliveredBlock.Tool("t3", "Edit SessionsScreen.kt", ToolState.COMPLETED, MessagePayload.Inline("-    items(listing.conversations) { listed ->\n+    items(listing.conversations, key = { it.conversation.runtimeId + \"/\" + it.conversation.id }) { listed ->"), null),
            DeliveredBlock.Tool("t4", "Bash ./gradlew :app:assembleDebug", ToolState.COMPLETED, MessagePayload.Inline("BUILD SUCCESSFUL in 41s"), null),
            DeliveredBlock.Text(reply),
        ),
    )

    private val turn = TranscriptTurn("turn1", "u1", listOf("a1"), "a1", 5u, 83_000uL, false, true, TurnOutcome.COMPLETED, null, null)
    private val items = listOf(TranscriptItem.Message("u1", "u1"), TranscriptItem.Work("turn1"), TranscriptItem.Message("reply:a1", "a1"))
    private val byId = messages.associateBy { it.id }

    val finished = ConversationState(
        conversation = ConversationSummary("c2", ConversationTitle.Native(header.title), now, now, "claude", header.cwd, true, true, null, null),
        entries = transcriptEntries(items, listOf(turn)) { byId[it] },
        loading = false,
        models = models,
        selection = models[0].selection,
        hasCurrent = true,
        currentModelName = "Claude Opus 5.5",
        efforts = efforts,
        currentEffort = "medium",
        permissionModes = permissionModes,
        currentPermissionMode = permissionModes[0],
        editable = mapOf("u1" to EditableMessage("u1", inPlace = true, deleteOnly = false, deleteFollowing = false, revision = "r1")),
        canFork = true,
        composerReady = true,
        canCompact = true,
    )

    private val runningMessages = messages + message("u2", MessageRole.USER, 1, DeliveredBlock.Text("顺便跑一下全部检查。")) + message(
        "a2", MessageRole.ASSISTANT, 0,
        DeliveredBlock.Tool("t5", "Bash scripts/build-android.sh --skip-native", ToolState.RUNNING, null, null),
    )
    private val runningById = runningMessages.associateBy { it.id }
    private val runningTurns = listOf(turn, TranscriptTurn("turn2", "u2", listOf("a2"), null, 0u, null, true, false, null, null, null))
    private val runningItems = items + listOf(TranscriptItem.Message("u2", "u2"), TranscriptItem.Work("turn2"))

    val running = finished.copy(
        entries = transcriptEntries(runningItems, runningTurns) { runningById[it] },
        runState = RunState.RUNNING,
        canCancel = true,
        interactions = listOf(
            RuntimeInteraction(
                "i1",
                InteractionKind.Approval(
                    "允许运行命令？", "scripts/build-android.sh --skip-native",
                    listOf(InteractionOption("allow", "允许"), InteractionOption("always", "本会话都允许"), InteractionOption("deny", "拒绝")),
                ),
            ),
        ),
    )
}
