package app.velune.android

import app.velune.bindings.BindingActiveConversation
import app.velune.bindings.BindingConfigurationSnapshot
import app.velune.bindings.BindingListedConversation
import app.velune.bindings.BindingRuntimeInstance
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
    val conversations: List<BindingListedConversation> = emptyList(),
    val runtimes: List<BindingRuntimeInstance> = emptyList(),
    /** Working directories of the device's conversations, for starting another in one. */
    val projects: List<String> = emptyList(),
    /** Running or waiting conversations by ID. */
    val active: Map<String, BindingActiveConversation> = emptyMap(),
    val loaded: Boolean = false,
    val failure: String? = null,
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
            val application = hub.application(peer)
            val (list, activity) = withContext(Dispatchers.IO) {
                application.list() to application.conversationActivity(activityCursor)
            }
            activityCursor = activity.cursor
            _listing.value = listing(list, activity.active)
        } catch (error: Exception) {
            _listing.value = _listing.value.copy(loaded = true, failure = error.describe())
        }
    }

    private fun listing(list: BindingConfigurationSnapshot, active: List<BindingActiveConversation>) = DeviceListing(
        conversations = list.conversations.filter { !it.archived && it.conversation.parentConversationId == null },
        runtimes = list.runtimeInstances.filter { it.enabled },
        projects = list.conversationProjects.map { it.path }.distinct(),
        active = active.associateBy { it.conversationId },
        loaded = true,
    )
}
