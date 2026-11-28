package app.velune.android

import app.velune.bindings.application.ActiveConversation
import app.velune.bindings.application.ConfigurationSnapshot
import app.velune.bindings.remote.PathKind
import app.velune.bindings.application.ListedConversation
import app.velune.bindings.application.RuntimeInstance
import app.velune.bindings.VeluneApplication
import app.velune.bindings.remote.ContactConfirmation
import app.velune.bindings.remote.RemoteServiceSnapshot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

private const val LIST_REFRESH_INTERVAL_MS = 30_000L
private const val LIST_RETRY_INTERVAL_MS = 5_000L

data class DeviceListing(
    val conversations: List<ListedConversation> = emptyList(),
    /** Enabled runtimes: the ones a new conversation can start on. */
    val runtimes: List<RuntimeInstance> = emptyList(),
    /** Every runtime instance by ID, disabled ones included, as the list labels them. */
    val runtimeLabels: Map<String, RuntimeLabel> = emptyMap(),
    /** Display names of projects that have one, by path. */
    val projectNames: Map<String, String> = emptyMap(),
    /** Known project folders, ordered by recent project activity. */
    val projects: List<String> = emptyList(),
    /** Running or waiting conversations by ID. */
    val active: Map<String, ActiveConversation> = emptyMap(),
    val loaded: Boolean = false,
    val failure: String? = null,
    /** The path in use, such as “中继 320 ms”: a relayed connection explains a slow device. */
    val path: String? = null,
)

internal fun RemoteServiceSnapshot.conversationContacts() =
    contacts.filter { it.controlAuthorized && it.confirmation is ContactConfirmation.Confirmed }

internal fun Pair<DeviceSummary, DeviceListing>.canStartConversation() =
    first.status.tone == Tone.Positive && second.failure == null && second.runtimes.isNotEmpty()

/** The conversation list of one controlled device, read while it is on screen. */
class DeviceWorkspace(private val hub: DeviceHub, val peer: String, private val scope: CoroutineScope) {
    private val _listing = MutableStateFlow(DeviceListing())
    val listing: StateFlow<DeviceListing> = _listing.asStateFlow()
    private var polling: Job? = null
    private val listRefresh = Mutex()
    private var activityCursor = 0uL

    fun resume() {
        if (polling?.isActive == true) return
        polling = scope.launch {
            // The first successful list read establishes the shared device proxy.
            while (!refresh()) delay(LIST_RETRY_INTERVAL_MS)
            launch {
                while (isActive) {
                    refreshActivity()
                    delay(if (_listing.value.active.isEmpty()) 5_000 else 2_000)
                }
            }
            var interval = LIST_REFRESH_INTERVAL_MS
            while (isActive) {
                delay(interval)
                interval = if (refresh()) LIST_REFRESH_INTERVAL_MS else LIST_RETRY_INTERVAL_MS
            }
        }
    }

    fun pause() {
        polling?.cancel()
        polling = null
    }

    suspend fun refresh(): Boolean = listRefresh.withLock {
        try {
            val (list, path) = hub.call(peer, NativeOperation.CONFIGURATION_READ) { application ->
                application.list() to path(application)
            }
            _listing.update { listing(list, it.active).copy(path = path) }
            true
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _listing.update { it.copy(loaded = true, failure = error.describe()) }
            false
        }
    }

    private suspend fun refreshActivity() {
        try {
            val activity = hub.call(peer, NativeOperation.READ) { it.conversationActivity(activityCursor) }
            activityCursor = activity.cursor
            _listing.update { it.copy(active = activity.active.associateBy { row -> row.conversationId }) }
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            // The next poll can still update the activity of the visible rows.
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

    private fun listing(list: ConfigurationSnapshot, active: Map<String, ActiveConversation>): DeviceListing {
        val projects = list.conversationProjects.sortedByDescending { it.latestActivityUnixMs ?: Long.MIN_VALUE }
        return DeviceListing(
            conversations = list.conversations.filter { !it.archived && it.conversation.parentConversationId == null },
            runtimes = list.runtimeInstances.filter { it.enabled },
            runtimeLabels = list.runtimeTypes.associateBy { it.id }.let { types ->
                list.runtimeInstances.associate { it.id to RuntimeLabel(it.name, types[it.typeId]?.familyId) }
            },
            projectNames = projects.flatMap { project ->
                project.name?.let { name -> (listOfNotNull(project.path, project.primaryFolder) + project.folders).map { it to name } }.orEmpty()
            }.toMap(),
            projects = projects.flatMap { listOfNotNull(it.primaryFolder) + it.folders }.distinct(),
            active = active,
            loaded = true,
        )
    }
}
