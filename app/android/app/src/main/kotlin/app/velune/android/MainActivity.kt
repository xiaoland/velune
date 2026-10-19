package app.velune.android

import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver

class MainActivity : ComponentActivity() {
    private val model: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            VeluneTheme {
                Surface(Modifier.fillMaxSize()) { App(model) }
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

@Composable
private fun VeluneTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = colors, content = content)
}

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
        Screen.Devices -> DevicesScreen(hub, open = { model.push(Screen.Device(it)) }, add = { model.push(Screen.AddDevice) })
        Screen.AddDevice -> AddDeviceScreen(hub, back = { model.back() }, paired = { model.showPaired(it) })
        is Screen.Device -> ConversationListScreen(
            hub = hub,
            workspace = model.workspace(screen.peer),
            back = { model.back() },
            switchDevice = { model.switchDevice(it) },
            open = { model.openConversation(screen.peer, it) },
        )
        is Screen.Conversation -> ConversationScreen(hub, screen.peer, screen.model, back = { model.back() })
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
