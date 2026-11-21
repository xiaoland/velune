package app.velune.android

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import app.velune.bindings.remote.*
import kotlinx.coroutines.launch
import java.util.UUID

private data class RelayAccessDraft(
    val id: String = UUID.randomUUID().toString(),
    val url: String = "",
    val configured: Boolean = false,
    val token: String = "",
    val remove: Boolean = false,
)

/** Credentials stay in this in-memory form until the core atomically saves them. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RemoteNetworkDialog(hub: DeviceHub, dismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val remote by hub.state.collectAsState()
    var loaded by remember { mutableStateOf<NetworkState?>(null) }
    var mode by remember { mutableIntStateOf(0) }
    var urls by remember { mutableStateOf("") }
    var credentials by remember { mutableStateOf(emptyList<RelayAccessDraft>()) }
    var busy by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    fun fill(state: NetworkState) {
        loaded = state
        mode = when (state.saved?.relay) { is RelayConfig.Custom -> 1; RelayConfig.Disabled -> 2; else -> 0 }
        urls = (state.saved?.relay as? RelayConfig.Custom)?.urls.orEmpty().joinToString("\n")
        credentials = state.saved?.credentials.orEmpty().map { RelayAccessDraft(url = it.url, configured = it.configured) }
    }
    fun perform(body: suspend () -> Unit) {
        scope.launch {
            busy = true; failure = null
            try { body() } catch (error: Exception) { failure = error.describe() }
            finally { busy = false }
        }
    }
    LaunchedEffect(hub) { perform { fill(hub.perform { it.readNetworkState() }) } }
    DisposableEffect(Unit) { onDispose { credentials = emptyList() } }
    AlertDialog(
        onDismissRequest = { if (!busy) dismiss() },
        title = {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("本机连接设置")
                failure?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error) }
                if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        },
        text = {
            Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("用于这台手机连接其他设备。两端均需配置受限中继的访问凭据；配对身份保持不变。地址查找继续使用现有设置。")
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    for ((value, label) in listOf(0 to "公共中继", 1 to "自定义中继", 2 to "关闭中继")) {
                        FilterChip(selected = mode == value, onClick = { mode = value }, enabled = !busy, label = { Text(label) })
                    }
                }
                if (mode == 1) OutlinedTextField(
                    urls, { urls = it }, label = { Text("中继地址，每行一个") }, placeholder = { Text("https://relay.example.com/") },
                    modifier = Modifier.fillMaxWidth(), minLines = 2, enabled = !busy,
                )
                if (mode == 2) Text("关闭中继后，需要能直连对方设备。")
                Text("中继访问凭据", style = MaterialTheme.typography.titleSmall)
                Text("凭据只发往指定地址。留空保留已有凭据；移除须勾选后保存。中继候选与访问资格分别设置。")
                for (entry in credentials) key(entry.id) {
                    OutlinedTextField(
                        entry.url, { value -> credentials = credentials.map { if (it.id == entry.id) it.copy(url = value) else it } },
                        label = { Text("认证服务地址") }, modifier = Modifier.fillMaxWidth(), singleLine = true,
                        enabled = !busy && !entry.configured && !entry.remove,
                    )
                    OutlinedTextField(
                        entry.token, { value -> credentials = credentials.map { if (it.id == entry.id) it.copy(token = value) else it } },
                        label = { Text(if (entry.configured) "新 Bearer 令牌（留空保持）" else "Bearer 令牌") },
                        modifier = Modifier.fillMaxWidth(), singleLine = true, visualTransformation = PasswordVisualTransformation(), enabled = !busy && !entry.remove,
                    )
                    Row {
                        Checkbox(checked = entry.remove, onCheckedChange = { value -> credentials = credentials.map { if (it.id == entry.id) it.copy(remove = value) else it } }, enabled = !busy)
                        TextButton(onClick = { credentials = credentials.map { if (it.id == entry.id) it.copy(remove = !it.remove) else it } }, enabled = !busy) { Text("移除此凭据") }
                    }
                }
                if (credentials.size < 64) TextButton(onClick = { credentials = credentials + RelayAccessDraft() }, enabled = !busy) { Text("添加凭据") }
                (remote?.network ?: loaded)?.let { Text(remoteNetworkStatus(it)) }
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    TextButton(onClick = { perform { hub.perform { it.retryNetwork() } } }, enabled = !busy && loaded != null) { Text("重试已保存设置") }
                    TextButton(onClick = { perform { fill(hub.perform { it.readNetworkState() }) } }, enabled = !busy) { Text("重新载入（丢弃编辑）") }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { perform {
                val relay = when (mode) {
                    1 -> RelayConfig.Custom(urls.lines().map { it.trim() }.filter { it.isNotEmpty() }).also { require(it.urls.isNotEmpty()) { "请填写至少一个中继地址" } }
                    2 -> RelayConfig.Disabled
                    else -> RelayConfig.DevelopmentPublic
                }
                val edits = credentials.mapNotNull { entry ->
                    if (entry.remove && !entry.configured) null else RelayCredentialEdit(entry.url.trim(), when {
                        entry.remove -> CredentialEdit.Remove
                        entry.token.isNotEmpty() -> CredentialEdit.Set(entry.token)
                        entry.configured -> CredentialEdit.Keep
                        else -> throw IllegalArgumentException("请填写新凭据的 Bearer 令牌，或移除该条目")
                    })
                }
                val revision = checkNotNull(loaded).saved?.revision ?: 0uL
                fill(hub.perform { it.applyRelaySettings(revision, relay, edits) })
            } }, enabled = !busy && loaded != null) { Text("保存并应用") }
        },
        dismissButton = { TextButton(onClick = dismiss, enabled = !busy) { Text("完成") } },
    )
}

private fun remoteNetworkStatus(state: NetworkState): String {
    val phase = when (state.phase) {
        NetworkApplicationPhase.UNAPPLIED -> "尚未应用"
        NetworkApplicationPhase.APPLYING -> "正在应用已保存设置"
        NetworkApplicationPhase.APPLIED -> "已保存的设置已应用；设施可达状态如下"
        NetworkApplicationPhase.FAILED -> "设置已保存，应用失败，可重试${state.errorCode?.let { "（$it）" }.orEmpty()}"
    }
    return (listOf(phase) + state.facilities.map {
        val kind = if (it.kind == FacilityKind.HOME_RELAY) "中继" else "地址查找"
        val status = when (it.reachability) {
            FacilityReachability.UNKNOWN -> "尚未确认"
            FacilityReachability.CONNECTING -> "连接中"
            FacilityReachability.REACHABLE -> "可达"
            FacilityReachability.UNAVAILABLE -> "不可达"
            FacilityReachability.AUTHENTICATION_DENIED -> "认证被拒绝"
        }
        "$kind ${it.url}：$status"
    }).joinToString("\n")
}
