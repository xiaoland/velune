package app.velune.android

import app.velune.bindings.application.ActiveConversation
import app.velune.bindings.application.ConfigurationSnapshot
import app.velune.bindings.remote.PathKind
import app.velune.bindings.application.ListedConversation
import app.velune.bindings.application.RuntimeInstance
import app.velune.bindings.VeluneApplication
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class DeviceListing(
    val conversations: List<ListedConversation> = emptyList(),
    /** Enabled runtimes: the ones a new conversation can start on. */
    val runtimes: List<RuntimeInstance> = emptyList(),
    /** Every runtime instance by ID, disabled ones included, as the list labels them. */
    val runtimeLabels: Map<String, RuntimeLabel> = emptyMap(),
    /** Display names of projects that have one, by path. */
    val projectNames: Map<String, String> = emptyMap(),
    /** Working directories of the device's conversations, for starting another in one. */
    val projects: List<String> = emptyList(),
    /** Running or waiting conversations by ID. */
    val active: Map<String, ActiveConversation> = emptyMap(),
    val loaded: Boolean = false,
    val failure: String? = null,
    /** The path in use, such as “中继 320 ms”: a relayed connection explains a slow device. */
    val path: String? = null,
)

/** The conversation list of one controlled device, read while it is on screen. */
class DeviceWorkspace(private val hub: DeviceHub, val peer: String, private val scope: CoroutineScope) {
    private val _listing = MutableStateFlow(DeviceListing())
    val listing: StateFlow<DeviceListing> = _listing.asStateFlow()
    private var polling: Job? = null
    private var activityCursor = 0uL

    fun resume() {
        if (polling?.isActive == true) return
        polling = scope.launch {
            while (isActive) {
                refresh()
                delay(if (_listing.value.active.isEmpty()) 5_000 else 2_000)
            }
        }
    }

    fun pause() {
        polling?.cancel()
        polling = null
    }

    suspend fun refresh() {
        try {
            val (list, activity) = hub.call(peer, NativeOperation.CONFIGURATION_READ) { application ->
                application.list() to application.conversationActivity(activityCursor)
            }
            val application = hub.application(peer)
            activityCursor = activity.cursor
            _listing.value = listing(list, activity.active).copy(path = path(application))
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _listing.value = _listing.value.copy(loaded = true, failure = error.describe())
        }
    }

    private fun path(application: VeluneApplication): String? {
        val path = runCatching { application.connectionStatus() }.getOrNull()?.paths?.firstOrNull { it.selected } ?: return null
        val kind = when (path.kind) {
            PathKind.DIRECT -> "直连"
            PathKind.RELAY -> "中继"
            PathKind.OTHER -> return null
        }
        return "$kind ${path.rttMicroseconds / 1_000uL} ms"
    }

    private fun listing(list: ConfigurationSnapshot, active: List<ActiveConversation>) = DeviceListing(
        conversations = list.conversations.filter { !it.archived && it.conversation.parentConversationId == null },
        runtimes = list.runtimeInstances.filter { it.enabled },
        runtimeLabels = list.runtimeTypes.associateBy { it.id }.let { types ->
            list.runtimeInstances.associate { it.id to RuntimeLabel(it.name, types[it.typeId]?.familyId) }
        },
        projectNames = list.conversationProjects.flatMap { project -> project.name?.let { name -> listOfNotNull(project.path to name, project.primaryFolder?.let { it to name }) } ?: emptyList() }.toMap(),
        projects = list.conversationProjects.mapNotNull { it.primaryFolder }.distinct(),
        active = active.associateBy { it.conversationId },
        loaded = true,
    )
}
