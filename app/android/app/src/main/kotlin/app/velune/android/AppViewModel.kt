package app.velune.android

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope

/** The screens on the back stack; a conversation screen owns its model. */
sealed interface Screen {
    /** The home: every paired device's conversations in one list. */
    data object Sessions : Screen
    data object Devices : Screen
    class AddDevice(initialInvitation: String? = null) : Screen {
        val draft = AddDeviceDraft(initialInvitation)
    }
    data object About : Screen
    data object Telemetry : Screen
    data class Legal(val document: LegalDocument) : Screen
    class AgentUri(val address: String, val peer: String?) : Screen
    class Conversation(val peer: String, val model: ConversationModel) : Screen
}

class AppViewModel(application: Application) : AndroidViewModel(application) {
    val startupFailure = (application as VeluneAndroid).initializationFailure
    val hub: DeviceHub? = if (startupFailure == null) (application as VeluneAndroid).hub else null
    private val workspaces = mutableMapOf<String, DeviceWorkspace>()
    // Retained across rotation; a recreated process must handle its launch URI again.
    var launchIntentHandled = false
    var stack by mutableStateOf(listOf<Screen>(Screen.Sessions))
        private set
    val screen: Screen get() = stack.last()
    /** The device the session list is narrowed to; null shows them all. */
    var deviceFilter by mutableStateOf<String?>(null)

    fun workspace(peer: String): DeviceWorkspace =
        workspaces.getOrPut(peer) { DeviceWorkspace(checkNotNull(hub), peer, viewModelScope) }

    fun push(screen: Screen) {
        stack = stack + screen
        selectShownDevice()
    }

    fun openConversation(peer: String, target: ConversationTarget) =
        push(Screen.Conversation(peer, ConversationModel(checkNotNull(hub), peer, target, viewModelScope)))

    /** The devices page opens a device as the session list narrowed to it. */
    fun showDevice(peer: String) {
        deviceFilter = peer
        stack = listOf(Screen.Sessions)
        selectShownDevice()
    }

    fun openAgentUri(address: String, peer: String? = null) {
        if (screen is Screen.AgentUri) back()
        push(Screen.AgentUri(address, peer))
    }

    /** A new invitation previews above the current screen, preserving its drafts and return path. */
    fun openPairingLink(address: String) = push(Screen.AddDevice(address))

    fun openResolvedAgentUri(peer: String, target: ConversationTarget) {
        back()
        openConversation(peer, target)
    }

    fun back(): Boolean {
        if (stack.size <= 1) return false
        (stack.last() as? Screen.Conversation)?.model?.pause()
        stack = stack.dropLast(1)
        selectShownDevice()
        return true
    }

    /** After pairing, the new device's conversations join the session list. */
    fun showPaired(peer: String) {
        if (hub?.state?.value?.outgoingPairings?.firstOrNull { it.peerId == peer }?.permission == app.velune.bindings.remote.PairingPermission.TELEMETRY_PROFILE_READ) {
            stack = listOf(Screen.Sessions, Screen.Telemetry); selectShownDevice(); return
        }
        val address = stack.filterIsInstance<Screen.AgentUri>().lastOrNull()?.address
        stack = if (address != null) listOf(Screen.Sessions, Screen.AgentUri(address, peer)) else listOf(Screen.Sessions)
        selectShownDevice()
    }

    /** The session list reports the devices it shows itself, as they come and go. */
    private fun selectShownDevice() {
        val peer = when (val shown = screen) {
            Screen.Sessions -> return
            is Screen.Conversation -> shown.peer
            is Screen.AgentUri -> shown.peer
            else -> null
        }
        hub?.select(peer)
    }
}
