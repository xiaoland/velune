package app.velune.android.preview

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.velune.android.ConversationTarget
import kotlinx.coroutines.CoroutineScope

internal enum class PreviewGroup(val title: String) {
    Navigation("入口"),
    Sessions("会话与设备"),
    Conversation("会话详情"),
    Settings("设置"),
}

internal class PreviewPage(val content: @Composable (PreviewGallery) -> Unit)

internal class PreviewScenario(
    val id: String,
    val title: String,
    val description: String,
    val group: PreviewGroup,
    val page: (PreviewGallery) -> PreviewPage,
)

internal object PreviewCatalog {
    const val HOME = "gallery"

    val scenarios = listOf(
        screen(HOME, "场景目录", "浏览并打开内置场景", PreviewGroup.Navigation) { GalleryDirectory(it) },
        screen("sessions", "聚合会话", "浏览多台设备的会话，筛选、搜索或直接新建", PreviewGroup.Sessions) {
            PreviewSessions(it, PreviewFixtures.listings, filterable = true)
        },
        screen("empty-sessions", "会话为空", "设备在线，尚无会话历史", PreviewGroup.Sessions) {
            PreviewSessions(it, PreviewFixtures.emptyListings)
        },
        screen("no-devices", "无配对设备", "查看首次连接提示", PreviewGroup.Sessions) {
            PreviewSessions(it, emptyList())
        },
        screen("devices", "设备列表", "查看直连、中继和离线设备", PreviewGroup.Sessions) { PreviewDevices(it) },
        conversation("conversation", "已完成会话", "切换执行配置、编辑与分叉，查看后台任务和 Agent 协作") {
            PreviewConversation(PreviewFixtures.macPeer, PreviewFixtures.completedConversation())
        },
        conversation("running", "运行中会话", "查看审批、工作过程和大纲，试用排队、介入及停止") {
            PreviewConversation(PreviewFixtures.macPeer, PreviewFixtures.runningConversation())
        },
        conversation("failed", "失败会话", "查看失败步骤并继续执行") {
            PreviewConversation(PreviewFixtures.windowsPeer, PreviewFixtures.failedConversation())
        },
        conversation("new", "新会话", "切换设备、运行时与项目，保留输入草稿") {
            PreviewConversation.newSession(it.deviceFilter)
        },
        screen("appearance", "外观与显示", "调节界面、消息、代码字号与显示密度", PreviewGroup.Settings) {
            PreviewAppearanceScreen(it)
        },
    )

    private val byId = scenarios.associateBy { it.id }
    private val aliases = mapOf("list" to "sessions")

    fun find(id: String?): PreviewScenario = byId[aliases[id] ?: id] ?: byId.getValue(HOME)

    private fun screen(
        id: String,
        title: String,
        description: String,
        group: PreviewGroup,
        render: @Composable (PreviewGallery) -> Unit,
    ) = PreviewScenario(id, title, description, group) { PreviewPage(render) }

    private fun conversation(
        id: String,
        title: String,
        description: String,
        open: (PreviewGallery) -> PreviewConversation,
    ) = PreviewScenario(id, title, description, PreviewGroup.Conversation) { gallery ->
        val conversation = open(gallery)
        PreviewPage { conversation.Content(it) }
    }
}

internal class PreviewGallery(
    screen: String?,
    val scope: CoroutineScope,
    val appearance: PreviewAppearance,
) {
    var deviceFilter by mutableStateOf<String?>(null)
    private var pages by mutableStateOf(listOf(PreviewCatalog.find(PreviewCatalog.HOME).page(this)))
    val current: PreviewPage get() = pages.last()
    val canGoBack: Boolean get() = pages.size > 1

    init {
        val initial = PreviewCatalog.find(screen)
        if (initial.id != PreviewCatalog.HOME) open(initial)
    }

    fun open(id: String) = open(PreviewCatalog.find(id))

    fun open(scenario: PreviewScenario) {
        pages = pages + scenario.page(this)
    }

    fun openSession(peer: String, target: ConversationTarget.Existing) {
        openConversation(PreviewConversation(peer, PreviewFixtures.listedConversation(peer, target.conversationId)))
    }

    fun openConversation(conversation: PreviewConversation) {
        pages = pages + PreviewPage { conversation.Content(it) }
    }

    fun back() {
        if (canGoBack) pages = pages.dropLast(1)
    }
}
