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
    data class Device(val peer: String) : Screen
    class Conversation(val peer: String, val model: ConversationModel) : Screen
}

class AppViewModel(application: Application) : AndroidViewModel(application) {
    val startupFailure = (application as VeluneAndroid).initializationFailure
    val hub: DeviceHub? = if (startupFailure == null) DeviceHub(application).also { it.start() } else null
    private val workspaces = mutableMapOf<String, DeviceWorkspace>()
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

    fun back(): Boolean {
        if (stack.size <= 1) return false
        (stack.last() as? Screen.Conversation)?.model?.pause()
        stack = stack.dropLast(1)
        selectShownDevice()
        return true
    }

    /** After pairing, the device opens in place of the pairing screen. */
    fun showPaired(peer: String) {
        stack = listOf(Screen.Devices, Screen.Device(peer))
        selectShownDevice()
    }

    private fun selectShownDevice() {
        val peer = when (val shown = screen) {
            is Screen.Device -> shown.peer
            is Screen.Conversation -> shown.peer
            else -> null
        }
        hub?.select(peer)
    }
}
