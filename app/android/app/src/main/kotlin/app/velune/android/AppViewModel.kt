package app.velune.android

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.util.UUID

/** The screens on the back stack; a conversation screen owns its model. */
sealed interface Screen {
    /** The home: every paired device's conversations in one list. */
    data object Sessions : Screen
    data object Devices : Screen
    class AddDevice(initialInvitation: String? = null) : Screen {
        val draft = AddDeviceDraft(initialInvitation)
    }
    data object About : Screen
    data object Appearance : Screen
    data object NetworkSettings : Screen
    data object Telemetry : Screen
    data class Legal(val document: LegalDocument) : Screen
    class AgentUri(val address: String, val peer: String?) : Screen
    class Conversation(val peer: String, val model: ConversationModel) : Screen
}

class AppViewModel(application: Application) : AndroidViewModel(application) {
    val startupFailure = (application as VeluneAndroid).initializationFailure
    val hub: DeviceHub? = if (startupFailure == null) (application as VeluneAndroid).hub else null
    val appearance = AppearancePreferences(application)
    private val newSessionDefaults = application.getSharedPreferences("new-session-defaults", Application.MODE_PRIVATE)
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

    fun openNewConversation() {
        if (screen != Screen.Sessions) return
        val devices = availableSessionDevices()
        val lastPeer = newSessionDefaults.getString("peer", null)
        val peer = listOfNotNull(deviceFilter, lastPeer).firstOrNull { it in devices } ?: devices.keys.firstOrNull() ?: return
        val listing = devices.getValue(peer)
        val runtime = newSessionDefaults.getString("runtimeId", null)?.takeIf { id ->
            peer == lastPeer && listing.runtimes.any { it.id == id }
        }
        val cwd = if (peer == lastPeer) newSessionDefaults.getString("cwd", null) else listing.projects.firstOrNull()
        val draftId = newSessionDefaults.getString("draftId", null).takeIf { peer == lastPeer }
        val target = ConversationTarget.New(runtime ?: listing.runtimes.first().id, cwd, draftId)
        rememberNewSession(peer, target)
        openConversation(peer, target)
    }

    fun changeNewSessionDevice(peer: String) {
        val current = screen as? Screen.Conversation ?: return
        val target = current.model.retargetableNewSession() ?: return
        if (peer == current.peer) return
        val listing = availableSessionDevices()[peer] ?: return
        val input = current.model.state.value
        // A separate remote draft keeps the destination's existing input intact.
        val hasDraft = input.draft.isNotEmpty() || input.attachments.isNotEmpty()
        val draftId = target.draftId ?: if (hasDraft) UUID.randomUUID().toString() else null
        val next = ConversationTarget.New(listing.runtimes.first().id, listing.projects.firstOrNull(), draftId)
        current.model.pause()
        val destination = ConversationModel(checkNotNull(hub), peer, next, viewModelScope, initialDraft = input.draft.takeIf { draftId != null })
        stack = stack.dropLast(1) + Screen.Conversation(peer, destination)
        rememberNewSession(peer, next)
    }

    fun changeNewSessionRuntime(runtimeId: String) {
        val current = screen as? Screen.Conversation ?: return
        val target = current.model.retargetableNewSession() ?: return
        if (workspace(current.peer).listing.value.runtimes.none { it.id == runtimeId }) return
        val next = target.copy(runtimeId = runtimeId)
        current.model.updateNewSession(next)
        rememberNewSession(current.peer, next)
    }

    fun changeNewSessionProject(cwd: String?) {
        val current = screen as? Screen.Conversation ?: return
        val target = current.model.retargetableNewSession() ?: return
        if (cwd != null && cwd !in workspace(current.peer).listing.value.projects) return
        val next = target.copy(cwd = cwd)
        current.model.updateNewSession(next)
        rememberNewSession(current.peer, next)
    }

    private fun availableSessionDevices(): Map<String, DeviceListing> {
        val snapshot = hub?.state?.value ?: return emptyMap()
        return snapshot.conversationContacts().mapNotNull { contact ->
            val listing = workspace(contact.peerId).listing.value
            val entry = deviceSummary(snapshot, contact, listing.path) to listing
            if (entry.canStartConversation()) contact.peerId to listing else null
        }.toMap()
    }

    private fun rememberNewSession(peer: String, target: ConversationTarget.New) {
        newSessionDefaults.edit()
            .putString("peer", peer)
            .putString("runtimeId", target.runtimeId)
            .putString("cwd", target.cwd)
            .putString("draftId", target.draftId)
            .apply()
    }

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
            is Screen.Conversation -> if (shown.model.target is ConversationTarget.New) return else shown.peer
            is Screen.AgentUri -> shown.peer
            else -> null
        }
        hub?.select(peer)
    }
}
