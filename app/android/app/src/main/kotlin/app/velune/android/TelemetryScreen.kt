package app.velune.android

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import app.velune.bindings.*
import app.velune.bindings.application.*
import app.velune.bindings.conversation.*
import app.velune.bindings.remote.*
import app.velune.bindings.telemetry.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext
import java.util.UUID
import kotlin.math.roundToInt

private data class TelemetryHeader(val id: String = UUID.randomUUID().toString(), val name: String = "authorization", val value: String = "")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TelemetryScreen(hub: DeviceHub, back: () -> Unit) {
    val remote by hub.state.collectAsState()
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<TelemetrySnapshot?>(null) }
    var preference by remember { mutableStateOf(TelemetryPreference.UNSET) }
    var endpoint by remember { mutableStateOf("") }
    var ratio by remember { mutableFloatStateOf(1f) }
    var headers by remember { mutableStateOf(emptyList<TelemetryHeader>()) }
    var source by remember { mutableStateOf("") }
    var invitation by remember { mutableStateOf<String?>(null) }
    var status by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    fun fill(profile: TelemetryProfile?) {
        endpoint = profile?.endpoint ?: state?.endpoint.orEmpty(); ratio = (profile?.sampleRatio ?: state?.sampleRatio ?: 1.0).toFloat()
        headers = profile?.headers?.entries?.sortedBy { it.key }?.map { TelemetryHeader(name = it.key, value = it.value) }.orEmpty()
    }
    fun profile(): TelemetryProfile? {
        if (endpoint.isBlank()) return null
        require(headers.map { it.name }.distinct().size == headers.size) { "认证字段名称不能重复" }
        return TelemetryProfile(endpoint.trim(), headers.associate { it.name to it.value }, ratio.toDouble())
    }
    fun perform(body: suspend () -> Unit) {
        scope.launch {
            busy = true; failure = null; status = null
            try { body() } catch (error: Exception) { failure = error.describe() }
            finally { busy = false }
        }
    }
    LaunchedEffect(hub) {
        try {
            val loaded = withContext(Dispatchers.IO) { hub.telemetry().binding.let { it.snapshot() to it.profileForEditing() } }
            state = loaded.first; preference = loaded.first.preference; fill(loaded.second)
        } catch (error: Exception) { failure = error.describe() }
    }
    LaunchedEffect(hub, busy) {
        while (isActive) {
            delay(2000)
            if (!busy) runCatching { withContext(Dispatchers.IO) { hub.telemetry().binding.snapshot() } }.onSuccess { state = it }
        }
    }
    DisposableEffect(Unit) { onDispose { headers = emptyList(); invitation = null } }
    Scaffold(topBar = { TopAppBar(title = { Text("遥测") }, navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } }) }) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item {
                Text("本机遥测", style = MaterialTheme.typography.titleMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    for ((value, label) in listOf(TelemetryPreference.UNSET to "未设置", TelemetryPreference.OFF to "关闭", TelemetryPreference.ON to "开启")) {
                        FilterChip(selected = preference == value, onClick = { preference = value }, label = { Text(label) }, enabled = !busy)
                    }
                }
                OutlinedTextField(endpoint, { endpoint = it }, label = { Text("OTLP/gRPC 地址") }, singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy)
                Text("新链路采样率 ${(ratio * 100).roundToInt()}%")
                Slider(ratio, { ratio = it }, enabled = !busy)
            }
            items(headers, key = { it.id }) { header ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    OutlinedTextField(header.name, { value -> headers = headers.map { if (it.id == header.id) it.copy(name = value) else it } }, label = { Text("Metadata 名称") }, modifier = Modifier.fillMaxWidth(), singleLine = true, enabled = !busy)
                    OutlinedTextField(header.value, { value -> headers = headers.map { if (it.id == header.id) it.copy(value = value) else it } }, label = { Text("认证值") }, modifier = Modifier.fillMaxWidth(), visualTransformation = PasswordVisualTransformation(), singleLine = true, enabled = !busy)
                    TextButton(onClick = { headers = headers.filterNot { it.id == header.id } }, enabled = !busy) { Text("移除字段") }
                }
            }
            item {
                if (headers.size < 16) TextButton(onClick = { headers = headers + TelemetryHeader() }, enabled = !busy) { Text("添加认证字段") }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { perform {
                        val profile = profile(); val expected = checkNotNull(state).revision; val selected = preference
                        state = withContext(Dispatchers.IO) { hub.telemetry().let { it.save(expected, selected, profile); it.binding.snapshot() } }
                        status = telemetryStatus(checkNotNull(state))
                    } }, enabled = !busy && state?.configurationAvailable == true) { Text("保存") }
                    OutlinedButton(onClick = { perform {
                        val profile = checkNotNull(profile()) { "请填写接收地址" }
                        val result = withContext(Dispatchers.IO) { hub.telemetry().binding.sendTest(profile) }
                        status = "Traces：${testResult(result.traces)}；Logs：${testResult(result.logs)}；Metrics：${testResult(result.metrics)}"
                    } }, enabled = !busy && endpoint.isNotBlank()) { Text("发送测试事件") }
                }
                if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                state?.let { Text(telemetryStatus(it), style = MaterialTheme.typography.bodyMedium) }
                status?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
            item {
                HorizontalDivider(); Text("从设备复制", style = MaterialTheme.typography.titleMedium)
                remote?.contacts?.filter { it.confirmation is ContactConfirmation.Confirmed }?.forEach { contact ->
                    FilterChip(selected = source == contact.peerId, onClick = { source = contact.peerId }, label = { Text(contact.name) })
                }
                Button(onClick = { perform {
                    val expected = checkNotNull(state).revision
                    val copied = hub.perform { checkNotNull(it.readTelemetryProfile(source)) { "来源设备尚未保存遥测配置" } }
                    state = withContext(Dispatchers.IO) { hub.telemetry().let { owner -> owner.save(expected, owner.binding.snapshot().preference, copied); owner.binding.snapshot() } }
                    preference = checkNotNull(state).preference; fill(copied)
                    val host = java.net.URI(copied.endpoint).host.orEmpty().removeSurrounding("[", "]")
                    status = if (host == "localhost" || host == "::1" || host.startsWith("127.")) "已复制；来源地址为回环地址，请调整并测试" else "已复制；请发送测试事件检查地址与认证"
                } }, enabled = !busy && source.isNotEmpty() && state?.configurationAvailable == true) { Text("复制遥测配置（含认证）") }
            }
            item {
                HorizontalDivider(); Text("允许读取本机配置（含认证）", style = MaterialTheme.typography.titleMedium)
                val names = buildMap {
                    remote?.contacts?.filter { it.confirmation is ContactConfirmation.Confirmed }?.forEach { put(it.peerId, it.name) }
                    remote?.incomingGrants?.forEach { put(it.peerId, it.name) }
                    remote?.telemetryProfileGrants?.forEach { put(it.peerId, it.name) }
                }
                names.forEach { (peer, name) ->
                    val allowed = remote?.telemetryProfileGrants?.any { it.peerId == peer } == true
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(name)
                        TextButton(onClick = { perform { hub.perform { if (allowed) it.revokeTelemetryProfile(peer) else it.allowTelemetryProfile(peer) } } }, enabled = !busy) { Text(if (allowed) "撤销" else "允许") }
                    }
                }
                OutlinedButton(onClick = { perform { invitation = hub.perform { it.createTelemetryInvitation(300uL).encoded } } }, enabled = !busy && remote != null) { Text("生成遥测分享邀请") }
                invitation?.let { encoded -> androidx.compose.foundation.text.selection.SelectionContainer { Text(encoded, style = MaterialTheme.typography.bodySmall) } }
            }
            items(remote?.incomingPairings?.filter { it.permission == PairingPermission.TELEMETRY_PROFILE_READ && it.phase == PairingPhase.AWAITING_APPROVAL }.orEmpty(), key = { it.attemptId }) { pairing ->
                LaunchedEffect(pairing.attemptId) { invitation = null }
                Text("${pairing.name} · 遥测配置（含认证）")
                Text(pairing.verificationCode.orEmpty(), style = MaterialTheme.typography.headlineMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { perform { hub.perform { it.approvePairing(pairing.attemptId, pairing.peerId) } } }, enabled = !busy) { Text("核对一致，允许") }
                    OutlinedButton(onClick = { perform { hub.perform { it.rejectPairing(pairing.attemptId, pairing.peerId) } } }, enabled = !busy) { Text("拒绝") }
                }
            }
        }
    }
}

private fun telemetryStatus(state: TelemetrySnapshot): String = when {
    state.status == "development_configuration_unavailable" -> "开发启动配置不可用，遥测未接入"
    !state.configurationAvailable -> "配置无法读取，遥测未启用；原文件已保留"
    !state.applied -> "已保存，尚未应用"
    state.rustActiveRevision == null -> "已关闭"
    state.rustExport.lastTransportResult == "transport_error" || state.nativeExport.lastTransportResult == "transport_error" -> "已应用，导出失败"
    state.developmentOverride -> "开发启动配置 · 已应用"
    else -> "已应用 · 修订 ${state.revision}"
}
private fun testResult(value: String): String = when (value) {
    "accepted" -> "接收端已接受"
    "partially_rejected" -> "部分拒绝"
    "unauthenticated", "permission_denied" -> "认证被拒绝"
    "timeout" -> "超时"
    "unavailable" -> "无法连接"
    else -> "被拒绝"
}
