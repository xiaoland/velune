package app.velune.android

import android.app.Application
import android.os.Bundle
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import app.velune.bindings.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI

/** Debug-only, independent echo ALPN. Never loads product identity, trust, or business state. */
class TransportProbeActivity : ComponentActivity() {
    private val model: TransportProbeViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                model.foreground(true)
                try { awaitCancellation() } finally { model.foreground(false) }
            }
        }
        setContent {
            val state by model.state.collectAsStateWithLifecycle()
            MaterialTheme(colorScheme = if (isSystemInDarkTheme()) darkColorScheme() else lightColorScheme()) {
                Scaffold { padding ->
                    Column(Modifier.fillMaxSize().padding(padding).padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        Text("S1 传输探针", style = MaterialTheme.typography.headlineSmall)
                        Text(state.status)
                        Text(state.path)
                        Text("成功 ${state.successes} · 失败 ${state.failures}")
                        Text("本次只连接一次；后台暂停 marker，回前台继续同一连接。窗口最长 300 秒。")
                        Text("地址和网络事件仅写入应用私有诊断文件；本页面不接入业务。")
                        Button(onClick = model::stop, enabled = !state.finished) { Text("停止探针") }
                    }
                }
            }
        }
    }
}

data class TransportProbeState(
    val status: String = "正在读取私有合成目标",
    val path: String = "尚无路径",
    val successes: Long = 0,
    val failures: Long = 0,
    val finished: Boolean = false,
)

class TransportProbeViewModel(application: Application) : AndroidViewModel(application) {
    private val mutable = MutableStateFlow(TransportProbeState())
    val state = mutable.asStateFlow()
    // This owned scope finishes native shutdown and evidence flushing after the Activity is destroyed.
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val active = MutableStateFlow(false)
    private val lifecycleSignals = Channel<Unit>(Channel.CONFLATED)
    private val events = Channel<EvidenceEvent>(1_024)
    private val networks = Channel<BindingPlatformNetworkSnapshot>(Channel.CONFLATED)
    private var writer: Job? = null
    private var networkReporter: Job? = null
    private var observation: Job? = null
    private var nativeObservation: Job? = null
    private var mdnsObservation: Job? = null
    private var deadline: Job? = null
    private var monitor: NetworkMonitor? = null
    private var probe: VeluneTransportProbe? = null
    private var connection: VeluneTransportProbeConnection? = null
    private var stopped = false
    private var evidenceFailed = false
    private var markerSequence = 0uL
    private var windowDeadlineMs: Long? = null
    private var mdnsEnabled = false
    private var multicastLock: ProbeMulticastLock? = null
    private val startup: Job

    private data class EvidenceEvent(val kind: String, val fields: JSONObject, val elapsedMs: Long)

    init {
        startup = scope.launch(start = CoroutineStart.LAZY) {
            var stage = ProbeStage.PLATFORM_INITIALIZATION
            try {
                check((application as VeluneAndroid).initializationFailure == null)
                stage = ProbeStage.EVIDENCE_OPEN
                val evidence = withContext(Dispatchers.IO) { ProbeEvidence(application.filesDir) }
                writer = scope.launch(Dispatchers.IO) {
                    try {
                        for (event in events) evidence.record(event.kind, event.fields, event.elapsedMs)
                    } catch (_: Exception) {
                        withContext(Dispatchers.Main) { failEvidence() }
                    }
                }
                stage = ProbeStage.TARGET_READ
                val targetText = withContext(Dispatchers.IO) { evidence.readTarget() }
                stage = ProbeStage.TARGET_PARSE
                val target = withContext(Dispatchers.IO) { ProbeTarget.parse(JSONObject(targetText)) }
                if (stopped) return@launch
                mdnsEnabled = target.mdns
                event("run_open", JSONObject().put("windowLimitMs", WINDOW_MS).put("publicInfrastructure", target.publicInfrastructure).put("mdns", target.mdns))
                stage = ProbeStage.ENDPOINT_OPEN
                probe = withContext(Dispatchers.IO) {
                    VeluneTransportProbe.open(BindingTransportProbeOptions(
                        relay = if (target.publicInfrastructure) BindingRelayConfig.DevelopmentPublic else BindingRelayConfig.Disabled,
                        discovery = if (target.publicInfrastructure) BindingDiscoveryConfig.DevelopmentPublic else BindingDiscoveryConfig.None,
                        pathPolicy = BindingRemotePathPolicy.AUTOMATIC,
                        bindAddresses = emptyList(),
                    ))
                }
                if (stopped) return@launch
                val local = checkNotNull(probe)
                stage = ProbeStage.NETWORK_MONITOR
                monitor = NetworkMonitor(application,
                    report = { networks.trySend(it) },
                    observed = { event("network_callback", it.evidence(), it.elapsedRealtimeMs.toLong()) },
                ).also { it.start() }
                networkReporter = scope.launch {
                    for (snapshot in networks) {
                        try {
                            val acknowledged = withContext(Dispatchers.IO) { local.reportNetworkChange(snapshot) }
                            event("network_report_ack", JSONObject().put("sequence", acknowledged.toString()).put("reportedSequence", snapshot.sequence.toString()))
                        } catch (cancelled: CancellationException) { throw cancelled }
                        catch (_: Exception) { event("network_report_failed", JSONObject().put("sequence", snapshot.sequence.toString())) }
                    }
                }
                if (active.value) monitor?.foreground()
                nativeObservation = scope.launch {
                    active.collectLatest { foreground ->
                        if (foreground && !stopped) nativeEvents(local)
                    }
                }
                if (target.mdns) {
                    while (!active.value && !stopped) lifecycleSignals.receive()
                    if (stopped || !setMulticastHeld(true, "mdns_start")) return@launch
                    event("mdns_enable_started")
                    stage = ProbeStage.MDNS_ENABLE
                    withContext(Dispatchers.IO) { local.enableMdns() }
                    event("mdns_enabled")
                    mdnsObservation = scope.launch {
                        active.collectLatest { foreground ->
                            if (foreground && !stopped) observeMdns(local, target.remote.endpointId)
                        }
                    }
                }
                event("connect_started")
                stage = ProbeStage.CONNECT
                // The only dial in this ViewModel; timeout and path failure never return here.
                connection = withContext(Dispatchers.IO) { local.connect(target.remote) }
                if (stopped) return@launch
                val current = checkNotNull(connection)
                event("connect_established")
                if (target.mdns) {
                    while (!active.value && !stopped) lifecycleSignals.receive()
                    if (stopped) return@launch
                    resolveMdns(local, target.remote.endpointId, "connected", 2_000uL)
                }
                if (stopped) return@launch
                val began = SystemClock.elapsedRealtime()
                windowDeadlineMs = began + WINDOW_MS
                event("window_started", JSONObject().put("deadlineElapsedRealtimeMs", windowDeadlineMs))
                mutable.update { it.copy(status = "已连接，正在记录同一连接") }
                deadline = scope.launch {
                    delay(WINDOW_MS)
                    finish("window_complete", "观察窗口已结束")
                }
                observation = scope.launch {
                    active.collectLatest { foreground ->
                        if (foreground && !stopped) coroutineScope {
                            launch { markers(current, target) }
                            launch { observe(current) }
                            awaitCancellation()
                        }
                    }
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: LinkageError) { startupFailed(stage, error) }
            catch (error: Exception) { startupFailed(stage, error) }
        }
        startup.start()
    }

    fun foreground(value: Boolean) {
        if (stopped) return
        if (windowExpired()) return
        if (mdnsEnabled && !setMulticastHeld(value, if (value) "foreground" else "background")) return
        active.value = value
        lifecycleSignals.trySend(Unit)
        event(if (value) "foreground" else "background")
        if (value) monitor?.foreground()
    }

    fun stop() = finish("user_stop", "探针已停止")

    private fun startupFailed(stage: ProbeStage, error: Throwable) {
        val failure = ProbeFailure.from(error)
        event("probe_failure", failure.evidence(stage))
        finish("${stage.code}_failed", failure.message(stage, mdnsEnabled))
    }

    private fun windowExpired(): Boolean {
        if (windowDeadlineMs?.let { SystemClock.elapsedRealtime() >= it } != true) return false
        finish("window_complete", "观察窗口已结束")
        return true
    }

    private fun setMulticastHeld(held: Boolean, reason: String): Boolean {
        try {
            if (held && multicastLock == null) multicastLock = ProbeMulticastLock(getApplication())
            val actual = multicastLock?.setHeld(held) ?: false
            event("multicast_lock", JSONObject().put("reason", reason).put("held", actual))
            check(actual == held)
            return true
        } catch (error: Exception) {
            evidenceFailed = true
            val failure = ProbeFailure.from(error)
            event("multicast_lock_failed", failure.evidence(ProbeStage.MULTICAST_LOCK).put("reason", reason).put("requestedHeld", held))
            finish("mdns_environment_failed", failure.message(ProbeStage.MULTICAST_LOCK, mdnsEnabled))
            return false
        }
    }

    private suspend fun markers(current: VeluneTransportProbeConnection, target: ProbeTarget) {
        val payload = ByteArray(target.payloadBytes) { (it % 251).toByte() }
        while (currentCoroutineContext().isActive && !stopped) {
            if (windowExpired()) return
            val sequence = ++markerSequence
            val started = SystemClock.elapsedRealtime()
            event("marker_started", JSONObject().put("sequence", sequence.toString()).put("payloadBytes", payload.size), started)
            try {
                val reply = withContext(Dispatchers.IO) { current.roundTrip(sequence, payload, target.timeoutMs.toULong()) }
                event("marker_reply", JSONObject()
                    .put("sequence", reply.sequence.toString()).put("payloadMatches", reply.payloadMatches)
                    .put("elapsedMicroseconds", reply.elapsedMicroseconds.toString())
                    .put("receivedBytes", reply.receivedBytes.toString())
                    .put("startedElapsedRealtimeMs", started))
                mutable.update { if (reply.payloadMatches && reply.sequence == sequence) it.copy(successes = it.successes + 1) else it.copy(failures = it.failures + 1) }
                if (!reply.payloadMatches || reply.sequence != sequence) {
                    finish("invalid_reply", "marker 校验失败，已停止")
                    return
                }
                val diagnostics = try { withContext(Dispatchers.IO) { current.diagnostics() } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) {
                    finish("path_observer_failed", "路径统计读取失败，本次证据不完整")
                    return
                }
                event("diagnostics", diagnostics.evidence())
                showPath(diagnostics.status)
                if (diagnostics.status.closed) {
                    finish("connection_closed", "原连接已关闭；不会自动重拨")
                    return
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                event("marker_failed", JSONObject().put("sequence", sequence.toString()).put("startedElapsedRealtimeMs", started))
                mutable.update { it.copy(failures = it.failures + 1) }
            }
            delay((target.intervalMs - (SystemClock.elapsedRealtime() - started)).coerceAtLeast(0))
        }
    }

    private suspend fun observe(current: VeluneTransportProbeConnection) {
        try {
            val observer = withContext(Dispatchers.IO) { current.observePaths() }
            try {
                val initial = withContext(Dispatchers.IO) { current.diagnostics() }
                event("diagnostics", initial.evidence())
                showPath(initial.status)
                if (initial.status.closed) {
                    finish("connection_closed", "原连接已关闭；不会自动重拨")
                    return
                }
                while (currentCoroutineContext().isActive && !stopped) {
                    val observation = withContext(Dispatchers.IO) { observer.next(1_000uL) } ?: continue
                    val status = observation.status
                    event("path_observed", status.evidence().put("missedEvents", observation.missedEvents.toString()))
                    showPath(status)
                    if (observation.missedEvents > 0uL) {
                        evidenceFailed = true
                        finish("path_history_gap", "路径事件有缺口，本次历史证据不完整")
                        return
                    }
                    val diagnostics = withContext(Dispatchers.IO) { current.diagnostics() }
                    event("diagnostics", diagnostics.evidence())
                    if (status.closed) {
                        finish("connection_closed", "原连接已关闭；不会自动重拨")
                        return
                    }
                }
            } finally { withContext(NonCancellable + Dispatchers.IO) { observer.close() } }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { finish("path_observer_failed", "路径观察失败，本次证据不完整") }
    }

    private suspend fun nativeEvents(local: VeluneTransportProbe) {
        try {
            while (currentCoroutineContext().isActive && !stopped) {
                val batch = withContext(Dispatchers.IO) { local.nativeEvents() }
                for (entry in batch) {
                    event("native_transport", JSONObject()
                        .put("sequence", entry.sequence.toString()).put("sampledAtMicros", entry.sampledAtMicros.toString())
                        .put("target", entry.target).put("droppedBefore", entry.droppedBefore.toString())
                        .put("fields", JSONArray(entry.fields.map { field -> JSONObject().put("name", field.name).put("value", field.value) })))
                    if (entry.droppedBefore > 0uL) {
                        failEvidence()
                        return
                    }
                }
                delay(500)
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { finish("native_observer_failed", "原生事件观察失败，本次证据不完整") }
    }

    private suspend fun observeMdns(local: VeluneTransportProbe, targetId: String) = coroutineScope {
        val resolutions = Channel<String>(64)
        launch {
            for (trigger in resolutions) resolveMdns(local, targetId, trigger, 1_000uL)
        }
        launch {
            while (isActive && !stopped) {
                if (resolutions.trySend("periodic").isFailure) failEvidence()
                delay(2_000)
            }
        }
        try {
            while (isActive && !stopped) {
                val batch = withContext(Dispatchers.IO) { local.mdnsEvents() }
                for (entry in batch) {
                    when (entry) {
                        is BindingProbeMdnsEvent.Discovered -> {
                            if (entry.endpointId != targetId) continue
                            event("mdns_discovered", JSONObject().put("sampledAtMicros", entry.sampledAtMicros.toString())
                                .put("endpointId", entry.endpointId).put("directAddresses", JSONArray(entry.directAddresses)))
                            if (resolutions.trySend("discovered").isFailure) failEvidence()
                        }
                        is BindingProbeMdnsEvent.Expired -> {
                            if (entry.endpointId != targetId) continue
                            event("mdns_expired", JSONObject().put("sampledAtMicros", entry.sampledAtMicros.toString())
                                .put("endpointId", entry.endpointId).put("directAddresses", JSONArray(entry.directAddresses)))
                            if (resolutions.trySend("expired").isFailure) failEvidence()
                        }
                        is BindingProbeMdnsEvent.Published -> {
                            event("mdns_published", JSONObject().put("sampledAtMicros", entry.sampledAtMicros.toString())
                                .put("directCount", entry.directCount.toString()).put("relayCount", entry.relayCount.toString()))
                        }
                        is BindingProbeMdnsEvent.Dropped -> {
                            event("mdns_dropped", JSONObject().put("count", entry.count.toString()))
                            failEvidence()
                        }
                    }
                }
                delay(500)
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { finish("mdns_observer_failed", "mDNS 事件观察失败，本次证据不完整") }
        finally { resolutions.close() }
    }

    private suspend fun resolveMdns(local: VeluneTransportProbe, targetId: String, trigger: String, timeout: ULong) {
        event("mdns_resolve_started", JSONObject().put("trigger", trigger).put("timeoutMs", timeout.toString()))
        try {
            val resolved = withContext(Dispatchers.IO) { local.mdnsResolve(targetId, timeout) }
            event("mdns_resolved", JSONObject().put("trigger", trigger).put("endpointId", resolved.endpointId)
                .put("directAddresses", JSONArray(resolved.directAddresses)).put("relayUrls", JSONArray(resolved.relayUrls)))
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) { event("mdns_resolve_failed", ProbeFailure.from(error).evidence(ProbeStage.MDNS_RESOLVE).put("trigger", trigger)) }
    }

    private fun showPath(status: BindingConnectionStatus) {
        val selected = status.paths.firstOrNull { it.selected }
        mutable.update { it.copy(path = "连接 ${status.connectionId} · ${selected?.kind ?: "待选路"} · 路径 ${selected?.pathId ?: "无"}") }
    }

    private fun event(kind: String, fields: JSONObject = JSONObject(), at: Long = SystemClock.elapsedRealtime()) {
        if (events.trySend(EvidenceEvent(kind, fields, at)).isFailure && !stopped) failEvidence()
    }

    private fun failEvidence() {
        if (evidenceFailed) return
        evidenceFailed = true
        finish("evidence_failed", "私有诊断记录失败，本次证据不完整")
    }

    private fun finish(reason: String, label: String) {
        if (stopped) return
        if (reason == "path_observer_failed" || reason == "native_observer_failed" || reason == "mdns_observer_failed") evidenceFailed = true
        stopped = true
        mutable.update { it.copy(status = label, finished = true) }
        if (mdnsEnabled) setMulticastHeld(false, "close")
        monitor?.close()
        monitor = null
        deadline?.cancel()
        active.value = false
        lifecycleSignals.trySend(Unit)
        scope.launch {
            startup.join()
            observation?.cancelAndJoin()
            nativeObservation?.cancelAndJoin()
            mdnsObservation?.cancelAndJoin()
            networkReporter?.cancelAndJoin()
            val shutdownComplete = withContext(Dispatchers.IO) {
                var complete = true
                try { connection?.shutdown() } catch (_: Exception) { complete = false }
                try { probe?.shutdown() } catch (_: Exception) { complete = false }
                try { connection?.close() } catch (_: Exception) { complete = false }
                try { probe?.close() } catch (_: Exception) { complete = false }
                complete
            }
            connection = null
            probe = null
            if (!shutdownComplete) {
                evidenceFailed = true
                mutable.update { it.copy(status = "探针关闭未完成，本次证据不完整") }
            }
            event(reason, JSONObject().put("evidenceComplete", !evidenceFailed).put("shutdownComplete", shutdownComplete))
            networks.close()
            lifecycleSignals.close()
            events.close()
            writer?.join()
            scope.cancel()
        }
    }

    override fun onCleared() {
        finish("activity_destroyed", "探针已关闭")
        super.onCleared()
    }

    companion object { private const val WINDOW_MS = 300_000L }
}

private data class ProbeTarget(
    val remote: BindingTransportProbeTarget,
    val publicInfrastructure: Boolean,
    val intervalMs: Long,
    val payloadBytes: Int,
    val timeoutMs: Long,
    val mdns: Boolean,
) {
    companion object {
        fun parse(json: JSONObject): ProbeTarget {
            val suppliedId = json.getString("endpointId")
            require(suppliedId.matches(Regex("[0-9a-fA-F]{64}")))
            val id = suppliedId.lowercase()
            fun strings(key: String): List<String> {
                val values = json.getJSONArray(key)
                require(values.length() <= 32)
                return List(values.length()) { values.getString(it).also { value -> require(value.length in 1..2_048 && value.none(Char::isISOControl)) } }
            }
            val relays = strings("relayUrls")
            val publicInfrastructure = json.getBoolean("publicInfrastructure")
            require(publicInfrastructure || relays.isEmpty())
            relays.forEach { value ->
                val uri = URI(value)
                require(uri.scheme == "https" && !uri.host.isNullOrEmpty() && uri.userInfo == null && uri.query == null && uri.fragment == null)
            }
            val interval = json.optLong("intervalMs", 1_000)
            val payload = json.optInt("payloadBytes", 256)
            val timeout = json.optLong("timeoutMs", 2_000)
            val direct = strings("directAddresses")
            val mdns = json.optBoolean("mdns", false)
            require(!mdns || (!publicInfrastructure && relays.isEmpty() && direct.isEmpty()))
            require(interval in 1_000..10_000 && payload in 1..1_024 && timeout in 100..10_000)
            return ProbeTarget(BindingTransportProbeTarget(id, relays, direct),
                publicInfrastructure, interval, payload, timeout, mdns)
        }
    }
}

private fun BindingPlatformNetworkSnapshot.evidence(): JSONObject = JSONObject()
    .put("sequence", sequence.toString()).put("reason", reason.name)
    .put("defaultNetworkId", defaultNetworkId ?: JSONObject.NULL)
    .put("networks", JSONArray(networks.map { network -> JSONObject()
        .put("id", network.id).put("wifi", network.wifi).put("cellular", network.cellular)
        .put("internet", network.internet).put("validated", network.validated)
        .put("interfaceName", network.interfaceName ?: JSONObject.NULL)
        .put("addresses", JSONArray(network.addresses)).put("dnsServers", JSONArray(network.dnsServers)) }))

private fun BindingConnectionStatus.evidence(): JSONObject = JSONObject()
    .put("connectionId", connectionId.toString()).put("sampledAtMicros", sampledAtMicros.toString()).put("closed", closed)
    .put("paths", JSONArray(paths.map { path -> JSONObject()
        .put("pathId", path.pathId.toString()).put("kind", path.kind.name).put("selected", path.selected)
        .put("addressFamily", path.addressFamily.name).put("source", path.source.name)
        .put("rttMicroseconds", path.rttMicroseconds.toString()).put("transmittedBytes", path.transmittedBytes.toString())
        .put("receivedBytes", path.receivedBytes.toString()).put("lostPackets", path.lostPackets.toString()) }))

private fun BindingConnectionDiagnostics.evidence(): JSONObject = JSONObject()
    .put("status", status.evidence())
    .put("addresses", JSONArray(paths.map { path -> JSONObject().put("pathId", path.pathId.toString())
        .put("localAddress", path.localAddress ?: JSONObject.NULL).put("remoteAddress", path.remoteAddress) }))
