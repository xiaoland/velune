package app.velune.android

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.velune.bindings.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.io.File

private const val VIEW_ID = "android-main"

data class RemoteUiState(
    val starting: Boolean = true,
    val busy: Boolean = false,
    val devices: List<BindingRemoteEndpointDescriptor> = emptyList(),
    val connected: BindingRemoteEndpointDescriptor? = null,
    val connection: String = "未连接",
    val configuration: BindingConfigurationSnapshot? = null,
    val runtimeId: String? = null,
    val models: List<BindingRuntimeModel> = emptyList(),
    val selectedModel: BindingModelSelection = BindingModelSelection.RuntimeDefault,
    val snapshot: BindingConversationSnapshot? = null,
    val expandedMessages: Map<String, BindingMessage> = emptyMap(),
    val pending: PendingOperation? = null,
    val pendingStatus: String = "",
    val error: String? = null,
    val info: String? = null,
)

class RemoteViewModel(application: Application) : AndroidViewModel(application) {
    private val mutable = MutableStateFlow(RemoteUiState())
    val state = mutable.asStateFlow()
    private val store = DeviceStore(application.noBackupFilesDir)
    private var saved = SavedDevices()
    private var endpoint: VeluneRemoteEndpoint? = null
    private var client: VeluneApplication? = null
    private var foreground = false
    private var poll: Job? = null
    private var generation = 0L
    private val connectivity = application.getSystemService(ConnectivityManager::class.java)
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = networkChanged()
        override fun onLost(network: Network) = networkChanged()
    }

    init {
        viewModelScope.launch {
            try {
                (application as VeluneAndroid).initializationFailure?.let { error(it) }
                saved = withContext(Dispatchers.IO) { store.load() }
                mutable.update { it.copy(devices = saved.devices, pending = saved.pending) }
                endpoint = withContext(Dispatchers.IO) {
                    val directory = File(application.noBackupFilesDir, "remote")
                    check(directory.isDirectory || directory.mkdirs()) { "无法创建私有设备目录。" }
                    VeluneRemoteEndpoint.open(
                        BindingRemoteOptions(
                            identityFile = File(directory, "identity").absolutePath,
                            trustFile = File(directory, "trust.json").absolutePath,
                            deviceName = "${Build.MANUFACTURER} ${Build.MODEL}",
                            relay = BindingRelayConfig.DevelopmentPublic,
                            discovery = BindingDiscoveryConfig.DevelopmentPublic,
                            pathPolicy = BindingRemotePathPolicy.AUTOMATIC,
                            bindAddresses = emptyList(),
                            requestTimeoutMs = 15_000uL,
                        ),
                        null,
                    )
                }
                connectivity.registerDefaultNetworkCallback(networkCallback)
            } catch (error: Exception) {
                showError(error)
            } finally {
                mutable.update { it.copy(starting = false) }
            }
        }
    }

    fun clearError() { mutable.update { it.copy(error = null, info = null) } }

    fun pair(invitation: String, consumed: () -> Unit) = action {
        val local = endpoint ?: error("远控核心尚未就绪。")
        val result = withContext(Dispatchers.IO) { local.pair(invitation.trim(), VIEW_ID) }
        consumed()
        adopt(result)
    }

    fun connect(descriptor: BindingRemoteEndpointDescriptor) = action {
        val local = endpoint ?: error("远控核心尚未就绪。")
        val result = withContext(Dispatchers.IO) { local.connect(descriptor, VIEW_ID) }
        adopt(result)
    }

    private suspend fun adopt(result: VeluneApplication) {
        val descriptor = withContext(Dispatchers.IO) { result.remoteDescriptor() }
        val previous = client
        generation += 1
        client = result
        saved = saved.copy(devices = saved.devices.filterNot { it.endpointId == descriptor.endpointId } + descriptor)
        withContext(Dispatchers.IO) { store.save(saved); previous?.shutdown() }
        mutable.update {
            it.copy(connected = descriptor, devices = saved.devices, snapshot = null,
                configuration = null, connection = "正在检查路径", error = null)
        }
        refresh(result)
        startPolling()
    }

    fun disconnect() = action {
        generation += 1
        poll?.cancel()
        val previous = client
        client = null
        mutable.update { it.copy(connected = null, configuration = null, snapshot = null, models = emptyList(), connection = "未连接") }
        withContext(Dispatchers.IO) { previous?.shutdown() }
    }

    fun forget(descriptor: BindingRemoteEndpointDescriptor) = action {
        check(saved.pending?.endpointId != descriptor.endpointId) { "请先处理这台设备的待确认操作。" }
        saved = saved.copy(devices = saved.devices.filterNot { it.endpointId == descriptor.endpointId })
        withContext(Dispatchers.IO) { store.save(saved) }
        mutable.update { it.copy(devices = saved.devices) }
    }

    fun refresh() = action { refresh(requireClient()) }

    private suspend fun refresh(current: VeluneApplication) {
        val configuration = withContext(Dispatchers.IO) { current.list() }
        if (current !== client) return
        val enabled = configuration.runtimeInstances.filter { it.enabled }
        val runtime = mutable.value.runtimeId?.takeIf { id -> enabled.any { it.id == id } }
            ?: configuration.selectedRuntimeInstanceId?.takeIf { id -> enabled.any { it.id == id } }
            ?: enabled.firstOrNull()?.id
        mutable.update { it.copy(configuration = configuration, runtimeId = runtime) }
        if (runtime != null) loadModels(current, runtime)
        updatePath(current)
    }

    fun selectRuntime(runtimeId: String) = action {
        mutable.update { it.copy(runtimeId = runtimeId, selectedModel = BindingModelSelection.RuntimeDefault, models = emptyList()) }
        loadModels(requireClient(), runtimeId)
    }

    private suspend fun loadModels(current: VeluneApplication, runtime: String) {
        val models = withContext(Dispatchers.IO) { current.runtimeModels(runtime) }
        if (current === client && runtime == mutable.value.runtimeId) {
            mutable.update { it.copy(models = models) }
        }
    }

    fun selectModel(selection: BindingModelSelection) {
        mutable.update { it.copy(selectedModel = selection) }
    }

    fun openConversation(conversation: BindingConversationSummary) = action {
        val current = requireClient()
        val snapshot = withContext(Dispatchers.IO) { current.openConversation(conversation.runtimeId, conversation.id).snapshot }
        if (current !== client) return@action
        mutable.update { it.copy(snapshot = snapshot, runtimeId = conversation.runtimeId, expandedMessages = emptyMap()) }
        loadModels(current, conversation.runtimeId)
    }

    fun loadMessage(messageId: String) = action {
        val snapshot = mutable.value.snapshot ?: return@action
        val current = requireClient()
        val messages = withContext(Dispatchers.IO) {
            current.messagePayloads(snapshot.conversation.runtimeId, snapshot.conversation.id, listOf(messageId))
        }
        if (current === client && mutable.value.snapshot?.conversation?.id == snapshot.conversation.id) {
            mutable.update { it.copy(expandedMessages = it.expandedMessages + messages.associateBy { message -> message.id }) }
        }
    }

    fun showConversations() { mutable.update { it.copy(snapshot = null) } }

    fun createConversation(cwd: String) {
        val state = mutable.value
        val runtime = state.runtimeId ?: return
        mutate("新建会话", runtime, null,
            BindingApplicationCommand.CreateConversationWithSelection(runtime, cwd.trim().takeIf { it.isNotEmpty() }, state.selectedModel))
    }

    fun send(text: String, accepted: () -> Unit) {
        val state = mutable.value
        val snapshot = state.snapshot ?: return
        val target = state.runtimeId ?: snapshot.contextRuntimeId
        mutate("发送消息", snapshot.conversation.runtimeId, snapshot.conversation.id,
            BindingApplicationCommand.SendTurnWithSelection(
                snapshot.conversation.runtimeId, snapshot.conversation.id, target,
                state.selectedModel, BindingComposedTurn(text, null, null, emptyList()),
            ), accepted)
    }

    fun cancel() {
        val snapshot = mutable.value.snapshot ?: return
        mutate("停止执行", snapshot.conversation.runtimeId, snapshot.conversation.id,
            BindingApplicationCommand.Cancel(snapshot.conversation.runtimeId, snapshot.conversation.id))
    }

    fun reply(interaction: BindingRuntimeInteraction, reply: BindingRuntimeInteractionReply) {
        val snapshot = mutable.value.snapshot ?: return
        mutate("回答交互", snapshot.conversation.runtimeId, snapshot.conversation.id,
            BindingApplicationCommand.ReplyRuntimeInteraction(snapshot.conversation.runtimeId, snapshot.conversation.id, interaction.id, reply))
    }

    private fun mutate(label: String, runtime: String?, conversation: String?, command: BindingApplicationCommand, accepted: () -> Unit = {}) = action {
        check(saved.pending == null) { "先查询待确认操作；应用不会自动重发。" }
        val current = requireClient()
        val device = mutable.value.connected ?: error("请先连接执行设备。")
        val requestId = withContext(Dispatchers.IO) { current.nextRequestId() }
        val pending = PendingOperation(device.endpointId, requestId, label, runtime, conversation)
        saved = saved.copy(pending = pending)
        // Persist before submission: process death must never turn an uncertain write into an automatic retry.
        withContext(Dispatchers.IO) { store.save(saved) }
        mutable.update { it.copy(pending = pending, pendingStatus = "等待执行端确认") }
        try {
            val status = withContext(Dispatchers.IO) { current.submitRequest(requestId, command) }
            handleStatus(status, current, accepted)
        } catch (error: Exception) {
            mutable.update { it.copy(pendingStatus = "连接中断或结果未知，请查询原请求。") }
            throw error
        }
    }

    fun queryPending() = action {
        val pending = saved.pending ?: return@action
        val current = requireClient()
        check(pending.endpointId == mutable.value.connected?.endpointId) { "请先连接操作所属的设备。" }
        val status = withContext(Dispatchers.IO) { current.queryRequest(pending.requestId) }
        handleStatus(status, current)
    }

    private suspend fun handleStatus(status: BindingRemoteRequestStatus, current: VeluneApplication, accepted: () -> Unit = {}) {
        when (status) {
            is BindingRemoteRequestStatus.Completed -> {
                val response = status.response
                if (response is BindingApplicationResponse.SnapshotResult && current === client) {
                    response.value.snapshot?.let { snapshot -> mutable.update { it.copy(snapshot = snapshot) } }
                }
                accepted()
                finishPending()
                mutable.update { it.copy(info = "操作已确认。") }
            }
            is BindingRemoteRequestStatus.Failed -> {
                finishPending()
                mutable.update { it.copy(error = status.failure.detail) }
            }
            BindingRemoteRequestStatus.Running -> mutable.update { it.copy(pendingStatus = "执行端已接纳，结果尚未返回。") }
            BindingRemoteRequestStatus.Unknown -> mutable.update { it.copy(pendingStatus = "执行端没有此请求的接纳证据。请检查会话后决定是否结束跟踪，不会自动重发。") }
            BindingRemoteRequestStatus.ResultExpired -> mutable.update { it.copy(pendingStatus = "回执已不可查询；请检查执行结果，不会自动重发。") }
            is BindingRemoteRequestStatus.TransportFailed -> mutable.update { it.copy(pendingStatus = "无法查询：${status.detail}") }
        }
    }

    fun stopTracking() = action {
        finishPending()
        mutable.update { it.copy(info = "已结束查询；这不会取消执行端已经接纳的操作。") }
    }

    private suspend fun finishPending() {
        saved = saved.copy(pending = null)
        withContext(Dispatchers.IO) { store.save(saved) }
        mutable.update { it.copy(pending = null, pendingStatus = "") }
    }

    fun foreground(active: Boolean) {
        foreground = active
        if (active) {
            networkChanged()
            startPolling()
        } else {
            poll?.cancel()
            poll = null
        }
    }

    private fun networkChanged() {
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { endpoint?.networkChange() } }
            catch (_: Exception) { /* The foreground refresh reports any connection failure. */ }
        }
    }

    private fun startPolling() {
        if (!foreground || client == null) return
        poll?.cancel()
        val current = client ?: return
        val epoch = generation
        poll = viewModelScope.launch {
            while (isActive && current === client && generation == epoch) {
                try {
                    val selected = mutable.value.snapshot?.conversation
                    if (selected != null) {
                        val snapshot = withContext(Dispatchers.IO) { current.snapshot(selected.runtimeId, selected.id).snapshot }
                        if (current === client && mutable.value.snapshot?.conversation?.id == selected.id) {
                            mutable.update { it.copy(snapshot = snapshot ?: it.snapshot) }
                        }
                    } else {
                        val configuration = withContext(Dispatchers.IO) { current.list() }
                        if (current === client) mutable.update { it.copy(configuration = configuration) }
                    }
                    updatePath(current)
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { mutable.update { it.copy(connection = "连接暂不可用，可断开后重新连接；执行端任务会继续。") } }
                delay(1_000)
            }
        }
    }

    private suspend fun updatePath(current: VeluneApplication) {
        val status = withContext(Dispatchers.IO) { current.connectionStatus() }
        if (current !== client) return
        val selected = status.paths.firstOrNull { it.selected }
        val path = when (selected?.kind) {
            BindingConnectionPathKind.DIRECT -> "Direct · 直连"
            BindingConnectionPathKind.RELAY -> "Relay · 中继"
            BindingConnectionPathKind.OTHER -> "其他路径"
            null -> "正在选择路径"
        }
        mutable.update { it.copy(connection = if (status.closed) "连接已关闭，请重新连接" else path) }
    }

    private fun requireClient() = client ?: error("请先连接执行设备。")

    private fun action(block: suspend () -> Unit) {
        if (mutable.value.busy) return
        viewModelScope.launch {
            mutable.update { it.copy(busy = true, error = null, info = null) }
            try { block() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { showError(error) }
            finally { mutable.update { it.copy(busy = false) } }
        }
    }

    private fun showError(error: Exception) {
        val message = when (error) {
            is BindingException.Diagnostic -> error.detail
            is BindingException.RemoteRequest -> error.detail
            is BindingException.Invalid -> error.detail
            is IllegalStateException -> error.message ?: "操作无法完成。"
            else -> "操作无法完成，请检查网络和执行端状态。"
        }
        mutable.update { it.copy(error = message) }
    }

    override fun onCleared() {
        poll?.cancel()
        try { connectivity.unregisterNetworkCallback(networkCallback) } catch (_: IllegalArgumentException) { }
        val local = endpoint
        client = null
        endpoint = null
        CoroutineScope(Dispatchers.IO).launch { try { local?.close() } catch (_: Exception) { } }
        super.onCleared()
    }
}
