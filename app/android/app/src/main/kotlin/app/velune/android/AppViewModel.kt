package app.velune.android

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope

/** The screens on the back stack; a conversation screen owns its model. */
sealed interface Screen {
    data object Devices : Screen
    data object AddDevice : Screen
    data object About : Screen
    data object Telemetry : Screen
    data class Legal(val document: LegalDocument) : Screen
    data class Device(val peer: String) : Screen
    class AgentUri(val address: String, val peer: String?) : Screen
    class Conversation(val peer: String, val model: ConversationModel) : Screen
}

class AppViewModel(application: Application) : AndroidViewModel(application) {
    val startupFailure = (application as VeluneAndroid).initializationFailure
    val hub: DeviceHub? = if (startupFailure == null) (application as VeluneAndroid).hub else null
    private val workspaces = mutableMapOf<String, DeviceWorkspace>()
    // Retained across rotation; a recreated process must handle its launch URI again.
    var launchIntentHandled = false
    var stack by mutableStateOf(listOf<Screen>(Screen.Devices))
        private set
    val screen: Screen get() = stack.last()

    fun workspace(peer: String): DeviceWorkspace =
        workspaces.getOrPut(peer) { DeviceWorkspace(checkNotNull(hub), peer, viewModelScope) }

    fun push(screen: Screen) {
        stack = stack + screen
        selectShownDevice()
    }

    /** Opens a device in place of the one shown: one interface, switched to another device. */
    fun switchDevice(peer: String) {
        stack = stack.takeWhile { it !is Screen.Device && it !is Screen.Conversation } + Screen.Device(peer)
        selectShownDevice()
    }

    fun openConversation(peer: String, target: ConversationTarget) =
        push(Screen.Conversation(peer, ConversationModel(checkNotNull(hub), peer, target, viewModelScope)))

    fun openAgentUri(address: String, peer: String? = null) {
        if (screen is Screen.AgentUri) back()
        push(Screen.AgentUri(address, peer))
    }

    fun openResolvedAgentUri(peer: String, target: ConversationTarget) {
        back()
        if (screen !is Screen.Conversation && screen != Screen.Device(peer)) switchDevice(peer)
        openConversation(peer, target)
    }

    fun back(): Boolean {
        if (stack.size <= 1) return false
        (stack.last() as? Screen.Conversation)?.model?.pause()
        stack = stack.dropLast(1)
        selectShownDevice()
        return true
    }

    /** After pairing, the device opens in place of the pairing screen. */
    fun showPaired(peer: String) {
        if (hub?.state?.value?.outgoingPairings?.firstOrNull { it.peerId == peer }?.permission == app.velune.bindings.remote.PairingPermission.TELEMETRY_PROFILE_READ) {
            stack = listOf(Screen.Devices, Screen.Telemetry); selectShownDevice(); return
        }
        val address = stack.filterIsInstance<Screen.AgentUri>().lastOrNull()?.address
        stack = if (address != null) listOf(Screen.Devices, Screen.AgentUri(address, peer))
            else listOf(Screen.Devices, Screen.Device(peer))
        selectShownDevice()
    }

    private fun selectShownDevice() {
        val peer = when (val shown = screen) {
            is Screen.Device -> shown.peer
            is Screen.Conversation -> shown.peer
            is Screen.AgentUri -> shown.peer
            else -> null
        }
        hub?.select(peer)
    }
}
