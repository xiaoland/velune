package app.velune.android

import android.content.Context
import android.os.Build
import android.provider.Settings
import app.velune.bindings.BindingDiscoveryConfig
import app.velune.bindings.BindingException
import app.velune.bindings.BindingPairingOfferInfo
import app.velune.bindings.BindingPairingStatus
import app.velune.bindings.BindingRelayConfig
import app.velune.bindings.BindingRemoteOptions
import app.velune.bindings.BindingRemotePathPolicy
import app.velune.bindings.BindingRemoteServiceSnapshot
import app.velune.bindings.VeluneApplication
import app.velune.bindings.VeluneRemoteEndpoint
import app.velune.bindings.decodeRemoteOffer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * This phone's remote service: pairing, the devices it controls, and their
 * applications. The phone runs no agents, so it opens the service without a
 * local application; every conversation lives on a controlled device.
 */
class DeviceHub(private val context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val _state = MutableStateFlow<BindingRemoteServiceSnapshot?>(null)
    val state: StateFlow<BindingRemoteServiceSnapshot?> = _state.asStateFlow()
    private val _failure = MutableStateFlow<String?>(null)
    val failure: StateFlow<String?> = _failure.asStateFlow()
    @Volatile private var endpoint: VeluneRemoteEndpoint? = null
    private val applications = ConcurrentHashMap<String, VeluneApplication>()
    /** One view per process: a device tells this phone's windows apart by it. */
    private val view = "android-" + UUID.randomUUID()
    private var network: NetworkMonitor? = null
    private var selected: String? = null
    private var foreground = false

    fun start() {
        scope.launch {
            try {
                // Identity and trusted devices stay out of backups: restoring them on another phone would clone it.
                val directory = File(context.noBackupFilesDir, "remote").apply { mkdirs() }
                val opened = VeluneRemoteEndpoint.open(
                    BindingRemoteOptions(
                        identityFile = File(directory, "device-identity").absolutePath,
                        stateFile = File(directory, "remote-state.json").absolutePath,
                        deviceName = deviceName(),
                        relay = BindingRelayConfig.DevelopmentPublic,
                        discovery = BindingDiscoveryConfig.DevelopmentPublic,
                        pathPolicy = BindingRemotePathPolicy.AUTOMATIC,
                        bindAddresses = emptyList(),
                        requestTimeoutMs = 30_000uL,
                    ),
                    null,
                )
                endpoint = opened
                _state.value = opened.readState()
                withContext(Dispatchers.Main) { startNetwork(opened) }
                val observer = opened.observeState()
                while (isActive) {
                    try {
                        observer.next(5_000uL)?.let { _state.value = it }
                    } catch (_: BindingException.Closed) {
                        return@launch
                    }
                }
            } catch (error: Exception) {
                _failure.value = "远程控制服务无法启动：${error.describe()}"
            }
        }
    }

    private fun startNetwork(opened: VeluneRemoteEndpoint) {
        network = NetworkMonitor(context, report = { snapshot ->
            scope.launch { runCatching { opened.reportNetworkChange(snapshot) } }
        }).also { it.start() }
    }

    private fun deviceName(): String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: Build.MODEL

    private fun opened(): VeluneRemoteEndpoint = endpoint ?: throw BindingException.Unavailable()

    /** Runs a call on the service off the main thread, then reads the state again. */
    suspend fun <T> perform(body: (VeluneRemoteEndpoint) -> T): T = withContext(Dispatchers.IO) {
        val opened = opened()
        body(opened).also { _state.value = opened.readState() }
    }

    fun decode(encoded: String): BindingPairingOfferInfo = decodeRemoteOffer(encoded.trim())

    suspend fun beginPairing(encoded: String): BindingPairingStatus = perform { it.beginPairing(encoded.trim()) }

    suspend fun cancelPairing(peer: String) = perform { it.cancelPairing(peer) }

    suspend fun forget(peer: String) {
        applications.remove(peer)
        perform { it.forgetDevice(peer) }
    }

    /** The application of a paired device: the same named methods as a device's own. */
    suspend fun application(peer: String): VeluneApplication = withContext(Dispatchers.IO) {
        applications[peer] ?: opened().connect(peer, view).also { applications[peer] = it }
    }

    /** Tells the core which device the user looks at, so it keeps that connection ready. */
    fun select(peer: String?) {
        val previous = selected
        selected = peer
        scope.launch {
            val opened = endpoint ?: return@launch
            if (previous != null && previous != peer) runCatching { opened.setControllerActivity(previous, false, false) }
            if (peer != null) runCatching { opened.setControllerActivity(peer, true, foreground) }
        }
    }

    fun setForeground(value: Boolean) {
        foreground = value
        if (value) network?.foreground()
        val peer = selected ?: return
        scope.launch { endpoint?.let { runCatching { it.setControllerActivity(peer, true, value) } } }
    }
}

/** A message the user can act on; details stay in the core's own diagnostics. */
fun Throwable.describe(): String = when (this) {
    is BindingException.Diagnostic -> detail
    is BindingException.RemoteRequest -> detail
    is BindingException.Invalid -> detail
    is BindingException.Unsupported -> detail
    is BindingException.Io -> detail
    is BindingException.Contract -> detail
    is BindingException.Closed -> "连接已关闭"
    is BindingException.Unavailable -> "远程控制服务不可用"
    else -> message ?: javaClass.simpleName
}
