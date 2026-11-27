package app.velune.android

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.remote.CredentialEdit
import app.velune.bindings.remote.FacilityKind
import app.velune.bindings.remote.FacilityReachability
import app.velune.bindings.remote.FacilityStatus
import app.velune.bindings.remote.NetworkApplicationPhase
import app.velune.bindings.remote.NetworkState
import app.velune.bindings.remote.RelayConfig
import app.velune.bindings.remote.RelayCredentialEdit
import kotlinx.coroutines.launch
import java.util.UUID

data class RelayAccessDraft(
    val id: String = UUID.randomUUID().toString(),
    val url: String = "",
    val configured: Boolean = false,
    val token: String = "",
    val remove: Boolean = false,
)

class RemoteNetworkActions(
    val back: () -> Unit,
    val selectMode: (Int) -> Unit,
    val editUrls: (String) -> Unit,
    val addCredential: () -> Unit,
    val updateCredential: (String, (RelayAccessDraft) -> RelayAccessDraft) -> Unit,
    val dropCredential: (String) -> Unit,
    val save: () -> Unit,
    val retry: () -> Unit,
    val reload: () -> Unit,
)

/** Credentials stay in this in-memory screen until the core atomically saves them. */
@Composable
fun RemoteNetworkScreen(hub: DeviceHub, back: () -> Unit) {
    val scope = rememberCoroutineScope()
    val remote by hub.state.collectAsState()
    var loaded by remember { mutableStateOf<NetworkState?>(null) }
    var mode by remember { mutableIntStateOf(0) }
    var urls by remember { mutableStateOf("") }
    var credentials by remember { mutableStateOf(emptyList<RelayAccessDraft>()) }
    var busy by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    var savedMessage by remember { mutableStateOf<String?>(null) }

    fun fill(state: NetworkState) {
        loaded = state
        mode = when (state.saved?.relay) {
            is RelayConfig.Custom -> 1
            RelayConfig.Disabled -> 2
            else -> 0
        }
        urls = (state.saved?.relay as? RelayConfig.Custom)?.urls.orEmpty().joinToString("\n")
        credentials = state.saved?.credentials.orEmpty().map { RelayAccessDraft(url = it.url, configured = it.configured) }
    }

    fun perform(body: suspend () -> Unit) {
        scope.launch {
            busy = true
            failure = null
            savedMessage = null
            try {
                body()
            } catch (error: Exception) {
                failure = error.describe()
            } finally {
                busy = false
            }
        }
    }

    LaunchedEffect(hub) { perform { fill(hub.perform { it.readNetworkState() }) } }
    DisposableEffect(Unit) { onDispose { credentials = emptyList() } }

    RemoteNetworkContent(
        status = remote?.network ?: loaded,
        loaded = loaded != null,
        mode = mode,
        urls = urls,
        credentials = credentials,
        busy = busy,
        failure = failure,
        savedMessage = savedMessage,
        actions = RemoteNetworkActions(
            back = { if (!busy) back() },
            selectMode = { mode = it; savedMessage = null },
            editUrls = { urls = it; savedMessage = null },
            addCredential = { if (credentials.size < 64) credentials = credentials + RelayAccessDraft() },
            updateCredential = { id, transform ->
                credentials = credentials.map { if (it.id == id) transform(it) else it }
                savedMessage = null
            },
            dropCredential = { id ->
                credentials = credentials.filterNot { it.id == id }
                savedMessage = null
            },
            save = {
                perform {
                    val relay = when (mode) {
                        1 -> RelayConfig.Custom(urls.lines().map { it.trim() }.filter { it.isNotEmpty() }).also {
                            require(it.urls.isNotEmpty()) { "请填写至少一个中继地址" }
                        }
                        2 -> RelayConfig.Disabled
                        else -> RelayConfig.DevelopmentPublic
                    }
                    val edits = credentials.mapNotNull { entry ->
                        if (entry.remove && !entry.configured) null
                        else {
                            val trimmed = entry.url.trim()
                            require(trimmed.isNotEmpty()) { "请填写认证服务地址，或移除空白凭据" }
                            RelayCredentialEdit(
                                trimmed,
                                when {
                                    entry.remove -> CredentialEdit.Remove
                                    entry.token.isNotEmpty() -> CredentialEdit.Set(entry.token)
                                    entry.configured -> CredentialEdit.Keep
                                    else -> throw IllegalArgumentException("请填写新凭据的 Bearer 令牌，或移除该条目")
                                },
                            )
                        }
                    }
                    val revision = checkNotNull(loaded).saved?.revision ?: 0uL
                    fill(hub.perform { it.applyRelaySettings(revision, relay, edits) })
                    savedMessage = "已保存并应用"
                }
            },
            retry = { perform { hub.perform { it.retryNetwork() } } },
            reload = { perform { fill(hub.perform { it.readNetworkState() }) } },
        ),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RemoteNetworkContent(
    status: NetworkState?,
    loaded: Boolean,
    mode: Int,
    urls: String,
    credentials: List<RelayAccessDraft>,
    busy: Boolean,
    failure: String?,
    savedMessage: String?,
    actions: RemoteNetworkActions,
) {
    val spacing = LocalAppearance.current.spacingScale
    Scaffold(
        topBar = {
            Column {
                TopAppBar(
                    navigationIcon = {
                        IconButton(onClick = actions.back, enabled = !busy) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回")
                        }
                    },
                    title = { Text("连接设置") },
                    actions = {
                        IconButton(onClick = actions.reload, enabled = !busy) {
                            Icon(Icons.Filled.Refresh, contentDescription = "重新载入")
                        }
                    },
                )
                if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            }
        },
        bottomBar = {
            Column(Modifier.fillMaxWidth().imePadding().navigationBarsPadding()) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp * spacing),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (savedMessage != null) {
                        Text(savedMessage, style = MaterialTheme.typography.bodySmall, color = Tone.Positive.color(), modifier = Modifier.weight(1f))
                    } else {
                        Spacer(Modifier.weight(1f))
                    }
                    Button(onClick = actions.save, enabled = !busy && loaded) {
                        Text("保存并应用")
                    }
                }
            }
        },
    ) { padding ->
        if (!loaded && status == null && failure == null) {
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@Scaffold
        }
        LazyColumn(
            Modifier.fillMaxSize().padding(padding),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp * spacing),
            verticalArrangement = Arrangement.spacedBy(16.dp * spacing),
        ) {
            failure?.let { error ->
                item(key = "failure") {
                    Surface(
                        color = MaterialTheme.colorScheme.errorContainer,
                        contentColor = MaterialTheme.colorScheme.onErrorContainer,
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Outlined.ErrorOutline, contentDescription = null, modifier = Modifier.size(18.dp))
                            Text(error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(start = 8.dp))
                        }
                    }
                }
            }
            status?.let { current ->
                item(key = "status") {
                    NetworkStatusCard(current, busy = busy, retry = actions.retry)
                }
            }
            item(key = "relay") {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp * spacing)) {
                    Text("中继模式", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    Text(
                        "用于这台手机中转连接其他设备；更改不影响已配对身份。",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    val options = listOf("公共中继", "自定义", "关闭")
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                        options.forEachIndexed { index, label ->
                            SegmentedButton(
                                selected = mode == index,
                                onClick = { actions.selectMode(index) },
                                enabled = !busy,
                                shape = SegmentedButtonDefaults.itemShape(index, options.size),
                                label = { Text(label, maxLines = 1) },
                            )
                        }
                    }
                    when (mode) {
                        1 -> OutlinedTextField(
                            value = urls,
                            onValueChange = actions.editUrls,
                            label = { Text("中继地址（每行一个）") },
                            placeholder = { Text("https://relay.example.com/") },
                            minLines = 2,
                            maxLines = 5,
                            enabled = !busy,
                            modifier = Modifier.fillMaxWidth(),
                        )
                        2 -> Text(
                            "关闭中继后，仅在可直连对方设备时可用。",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            item(key = "credentials-header") {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("访问凭据", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary, modifier = Modifier.weight(1f))
                        if (credentials.size < 64) {
                            TextButton(onClick = actions.addCredential, enabled = !busy) {
                                Icon(Icons.Filled.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                                Text("添加", modifier = Modifier.padding(start = 4.dp))
                            }
                        }
                    }
                    Text(
                        "仅受限中继需要；凭据只发往指定地址，已有令牌不回显，留空即保持。",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            if (credentials.isEmpty()) {
                item(key = "credentials-empty") {
                    Text(
                        "未配置访问凭据",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                items(credentials, key = { it.id }) { entry ->
                    CredentialCard(
                        entry = entry,
                        busy = busy,
                        onUpdate = { transform -> actions.updateCredential(entry.id, transform) },
                        onDrop = { actions.dropCredential(entry.id) },
                    )
                }
            }
        }
    }
}

@Composable
private fun NetworkStatusCard(state: NetworkState, busy: Boolean, retry: () -> Unit) {
    val (phaseTone, phaseText) = when (state.phase) {
        NetworkApplicationPhase.UNAPPLIED -> Tone.Neutral to "设置尚未应用"
        NetworkApplicationPhase.APPLYING -> Tone.Attention to "正在应用设置…"
        NetworkApplicationPhase.APPLIED -> Tone.Positive to "设置已应用"
        NetworkApplicationPhase.FAILED -> Tone.Negative to "应用失败${state.errorCode?.let { "（$it）" }.orEmpty()}"
    }
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                StatusDot(phaseTone, size = 8.dp)
                Text(phaseText, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(start = 8.dp).weight(1f))
                if (state.phase == NetworkApplicationPhase.FAILED || state.phase == NetworkApplicationPhase.UNAPPLIED) {
                    TextButton(onClick = retry, enabled = !busy) { Text("重试") }
                }
            }
            if (state.facilities.isNotEmpty()) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                state.facilities.forEach { facility -> FacilityRow(facility) }
            }
        }
    }
}

@Composable
private fun FacilityRow(facility: FacilityStatus) {
    val kind = if (facility.kind == FacilityKind.HOME_RELAY) "中继" else "地址查找"
    val (tone, label) = when (facility.reachability) {
        FacilityReachability.UNKNOWN -> Tone.Neutral to "待确认"
        FacilityReachability.CONNECTING -> Tone.Attention to "连接中"
        FacilityReachability.REACHABLE -> Tone.Positive to "可达"
        FacilityReachability.UNAVAILABLE -> Tone.Negative to "不可达"
        FacilityReachability.AUTHENTICATION_DENIED -> Tone.Negative to "认证被拒"
    }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        StatusDot(tone, size = 7.dp)
        Column(Modifier.weight(1f)) {
            Text(kind, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(facility.url, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Text(label, style = MaterialTheme.typography.labelMedium, color = tone.color())
    }
}

@Composable
private fun CredentialCard(
    entry: RelayAccessDraft,
    busy: Boolean,
    onUpdate: ((RelayAccessDraft) -> RelayAccessDraft) -> Unit,
    onDrop: () -> Unit,
) {
    var showToken by remember(entry.id) { mutableStateOf(false) }
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        shape = RoundedCornerShape(14.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        if (entry.configured) entry.url else "新凭据",
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Medium,
                        textDecoration = if (entry.remove) TextDecoration.LineThrough else null,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (entry.configured) {
                        Text(
                            if (entry.remove) "保存后将移除此凭据" else "已配置令牌",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (entry.remove) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                if (entry.configured) {
                    IconButton(onClick = { onUpdate { it.copy(remove = !it.remove) } }, enabled = !busy) {
                        Icon(
                            if (entry.remove) Icons.AutoMirrored.Outlined.Undo else Icons.Outlined.DeleteOutline,
                            contentDescription = if (entry.remove) "撤销移除" else "移除此凭据",
                        )
                    }
                } else {
                    IconButton(onClick = onDrop, enabled = !busy) {
                        Icon(Icons.Filled.Close, contentDescription = "删除此条")
                    }
                }
            }
            if (!entry.configured) {
                OutlinedTextField(
                    value = entry.url,
                    onValueChange = { value -> onUpdate { it.copy(url = value) } },
                    label = { Text("认证服务地址") },
                    placeholder = { Text("https://relay.example.com/") },
                    singleLine = true,
                    enabled = !busy,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            if (!entry.remove) {
                OutlinedTextField(
                    value = entry.token,
                    onValueChange = { value -> onUpdate { it.copy(token = value) } },
                    label = { Text(if (entry.configured) "新 Bearer 令牌（留空保持）" else "Bearer 令牌") },
                    singleLine = true,
                    enabled = !busy,
                    visualTransformation = if (showToken) VisualTransformation.None else PasswordVisualTransformation(),
                    trailingIcon = {
                        IconButton(onClick = { showToken = !showToken }) {
                            Icon(
                                if (showToken) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                contentDescription = if (showToken) "隐藏令牌" else "显示令牌",
                            )
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}
