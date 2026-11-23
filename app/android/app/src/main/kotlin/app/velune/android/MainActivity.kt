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
import androidx.compose.runtime.remember
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.lifecycle.compose.LocalLifecycleOwner
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
            VeluneTheme {
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
    BackHandler(enabled = model.stack.size > 1) { model.back() }
    when (val screen = model.screen) {
        Screen.Devices -> DevicesScreen(hub, open = { model.push(Screen.Device(it)) }, add = { model.push(Screen.AddDevice()) }, about = { model.push(Screen.About) }, telemetry = { model.push(Screen.Telemetry) })
        Screen.Telemetry -> TelemetryScreen(hub, back = { model.back() })
        is Screen.AddDevice -> key(screen) {
            AddDeviceScreen(hub, back = { model.back() }, paired = { model.showPaired(it) }, draft = screen.draft)
        }
        Screen.About -> AboutScreen(back = { model.back() }, open = { model.push(Screen.Legal(it)) })
        is Screen.Legal -> LegalDocumentScreen(screen.document, back = { model.back() })
        is Screen.Device -> ConversationListScreen(
            hub = hub,
            workspace = model.workspace(screen.peer),
            back = { model.back() },
            switchDevice = { model.switchDevice(it) },
            open = { model.openConversation(screen.peer, it) },
        )
        is Screen.AgentUri -> key(screen) {
            AgentUriScreen(hub, screen.address, screen.peer, back = { model.back() }, addDevice = { model.push(Screen.AddDevice()) }, open = model::openResolvedAgentUri)
        }
        is Screen.Conversation -> {
            val listing by model.workspace(screen.peer).listing.collectAsState()
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
                ConversationScreen(hub, screen.peer, listing.runtimeLabels[screen.model.target.runtimeId], screen.model, back = { model.back() })
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
