package app.velune.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver

class MainActivity : ComponentActivity() {
    private val model: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        (application as VeluneAndroid).configureDevelopment(intent)
        if (!model.launchIntentHandled) {
            model.launchIntentHandled = true
            openIncomingUri(intent)
        }
        enableEdgeToEdge()
        setContent {
            val appearance by model.appearance.values.collectAsStateWithLifecycle()
            VeluneTheme(appearance) {
                Surface(Modifier.fillMaxSize()) { App(model) }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        openIncomingUri(intent)
    }

    private fun openIncomingUri(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        val address = intent.data ?: return
        when {
            address.scheme == "agents" -> model.openAgentUri(address.toString())
            address.isPairingInvitation() -> {
                // Invitations belong only to the in-memory preview, never Activity state.
                intent.data = null
                model.openPairingLink(address.toString())
            }
        }
    }

    override fun onStart() {
        super.onStart()
        model.hub?.setForeground(true)
    }

    override fun onStop() {
        model.hub?.setForeground(false)
        super.onStop()
    }
}

private fun Uri.isPairingInvitation(): Boolean =
    scheme == "velune" && host == "pair" && path?.startsWith("/v3/") == true

@Composable
private fun App(model: AppViewModel) {
    val hub = model.hub
    if (hub == null) {
        Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
            Text(model.startupFailure ?: "无法初始化远控核心。")
        }
        return
    }
    val navigationState = rememberSaveableStateHolder()
    BackHandler(enabled = model.stack.size > 1) { model.back() }
    when (val screen = model.screen) {
        Screen.Sessions -> navigationState.SaveableStateProvider("sessions") {
            SessionsScreen(
                hub, workspace = model::workspace, filter = model.deviceFilter,
                actions = SessionsActions(
                    open = model::openConversation,
                    refresh = {},
                    filter = { model.deviceFilter = it },
                    devices = { model.push(Screen.Devices) },
                    add = { model.push(Screen.AddDevice()) },
                    telemetry = { model.push(Screen.Telemetry) },
                    about = { model.push(Screen.About) },
                    appearance = { model.push(Screen.Appearance) },
                    newSession = model::openNewConversation,
                ),
            )
        }
        Screen.Devices -> DevicesScreen(hub, back = { model.back() }, open = model::showDevice, add = { model.push(Screen.AddDevice()) })
        Screen.Telemetry -> TelemetryScreen(hub, back = { model.back() })
        is Screen.AddDevice -> key(screen) {
            AddDeviceScreen(hub, back = { model.back() }, paired = { model.showPaired(it) }, draft = screen.draft)
        }
        Screen.About -> AboutScreen(back = { model.back() }, open = { model.push(Screen.Legal(it)) })
        Screen.Appearance -> AppearanceScreen(model.appearance, back = { model.back() })
        is Screen.Legal -> LegalDocumentScreen(screen.document, back = { model.back() })
        is Screen.AgentUri -> key(screen) {
            AgentUriScreen(hub, screen.address, screen.peer, back = { model.back() }, addDevice = { model.push(Screen.AddDevice()) }, open = model::openResolvedAgentUri)
        }
        is Screen.Conversation -> key(screen.model) {
            val target = screen.model.target
            val workspace = model.workspace(screen.peer)
            val listing by workspace.listing.collectAsState()
            val newSession = if (target is ConversationTarget.New) {
                val remote by hub.state.collectAsState()
                val snapshot = remote
                val contacts = snapshot?.conversationContacts().orEmpty()
                val peers = contacts.mapTo(mutableSetOf()) { it.peerId }
                LaunchedEffect(peers) { hub.view(peers) }
                val devices = contacts.map { contact ->
                    key(contact.peerId) {
                        val workspace = model.workspace(contact.peerId)
                        val listing by workspace.listing.collectAsState()
                        WhileVisible(workspace, resume = workspace::resume, pause = workspace::pause)
                        deviceSummary(checkNotNull(snapshot), contact, listing.path) to listing
                    }
                }
                NewSessionContext(
                    screen.peer, target, devices,
                    model::changeNewSessionDevice, model::changeNewSessionRuntime, model::changeNewSessionProject,
                )
            } else {
                LaunchedEffect(screen.peer) { hub.select(screen.peer) }
                WhileVisible(workspace, resume = workspace::resume, pause = workspace::pause)
                null
            }
            val fallback = LocalUriHandler.current
            val links = remember(screen, fallback) {
                object : UriHandler {
                    override fun openUri(uri: String) {
                        val address = Uri.parse(uri)
                        when {
                            address.scheme == "agents" -> model.openAgentUri(uri, screen.peer)
                            address.isPairingInvitation() -> model.openPairingLink(uri)
                            else -> fallback.openUri(uri)
                        }
                    }
                }
            }
            CompositionLocalProvider(LocalUriHandler provides links) {
                ConversationScreen(
                    hub, screen.peer, listing, screen.model,
                    back = { model.back() },
                    openConversation = { target -> if (model.screen === screen) model.openConversation(screen.peer, target) },
                    newSession = newSession,
                )
            }
        }
    }
}

/** Runs `resume` while the screen is shown and the app is in the foreground. */
@Composable
fun WhileVisible(key: Any, resume: () -> Unit, pause: () -> Unit) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(key, lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> resume()
                Lifecycle.Event.ON_STOP -> pause()
                else -> {}
            }
        }
        lifecycle.addObserver(observer)
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) resume()
        onDispose {
            lifecycle.removeObserver(observer)
            pause()
        }
    }
}
