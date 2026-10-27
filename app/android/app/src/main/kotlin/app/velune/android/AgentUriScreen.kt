package app.velune.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingAgentUriResolution
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentUriScreen(hub: DeviceHub, address: String, peer: String?, back: () -> Unit, addDevice: () -> Unit, open: (String, ConversationTarget) -> Unit) {
    val state by hub.state.collectAsState()
    val devices = state?.let { snapshot -> snapshot.contacts.filter { it.controlAuthorized }.map { deviceSummary(snapshot, it) }.filter { it.confirmed } }.orEmpty()
    var selected by rememberSaveable { mutableStateOf(peer) }
    val target = selected ?: devices.singleOrNull()?.peerId
    var loading by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<BindingAgentUriResolution?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(target) {
        if (target == null) return@LaunchedEffect
        hub.select(target)
        loading = true
        result = null
        failure = null
        try {
            val found = hub.call(target) { it.resolveAgentUri(address, null) }
            result = found
            if (found.conversations.size == 1 && found.failures.isEmpty()) {
                val conversation = found.conversations.single().conversation
                open(target, ConversationTarget.Existing(conversation.runtimeId, conversation.id, conversation.title))
            } else if (found.conversations.isEmpty() && found.failures.isEmpty()) failure = "这台设备的已配置实例中找不到该会话。"
        } catch (error: CancellationException) { throw error }
        catch (error: Exception) { failure = error.describe() }
        finally { loading = false }
    }
    Scaffold(topBar = {
        TopAppBar(title = { Text(if (target == null && devices.isNotEmpty()) "选择设备" else "打开会话") }, navigationIcon = {
            IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") }
        })
    }) { padding ->
        LazyColumn(Modifier.fillMaxSize().padding(padding).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (state == null || loading) item { CircularProgressIndicator() }
            else if (target == null) {
                if (devices.isEmpty()) item { Button(onClick = addDevice) { Text("添加设备") } }
                items(devices, key = { it.peerId }) { device ->
                    Text(device.name, style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.fillMaxWidth().clickable { selected = device.peerId }.padding(vertical = 16.dp))
                }
            }
            failure?.let { detail -> item { Banner(detail) } }
            result?.let { found ->
                if (found.failures.isNotEmpty()) item { Banner(found.failures.joinToString("\n") { "${it.runtimeId}：${it.detail}" }) }
                items(found.conversations, key = { it.conversation.runtimeId + "/" + it.conversation.id }) { item ->
                    val conversation = item.conversation
                    Column(Modifier.fillMaxWidth().clickable {
                        target?.let { open(it, ConversationTarget.Existing(conversation.runtimeId, conversation.id, conversation.title)) }
                    }.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(conversation.title, style = MaterialTheme.typography.titleMedium)
                        Text(conversation.runtimeId, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        conversation.cwd?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                }
            }
        }
    }
}
