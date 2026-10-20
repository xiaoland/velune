package app.velune.android

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingContactConfirmation
import app.velune.bindings.BindingControllerPhase
import app.velune.bindings.BindingDeviceContact
import app.velune.bindings.BindingPairingPhase
import app.velune.bindings.BindingRemoteServiceSnapshot
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesScreen(hub: DeviceHub, open: (String) -> Unit, add: () -> Unit) {
    val state by hub.state.collectAsState()
    val failure by hub.failure.collectAsState()
    var removing by remember { mutableStateOf<BindingDeviceContact?>(null) }
    val scope = rememberCoroutineScope()
    Scaffold(
        topBar = { TopAppBar(title = { Text("设备") }) },
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = add,
                icon = { Icon(Icons.Filled.Add, contentDescription = null) },
                text = { Text("添加设备") },
            )
        },
    ) { padding ->
        val contacts = state?.contacts.orEmpty()
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                failure != null -> Message(failure!!)
                state == null -> Message("正在启动远程控制…")
                contacts.isEmpty() -> Message("还没有可控制的设备。\n在 Mac 或 Windows 的“设备”设置中选择“添加控制设备”，再扫描显示的二维码。")
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(contacts, key = { it.peerId }) { contact ->
                        DeviceRow(contact, deviceStatus(state!!, contact), open = { open(contact.peerId) }, remove = { removing = contact })
                    }
                }
            }
        }
    }
    removing?.let { contact ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("移除“${contact.name}”？") },
            text = { Text("这台手机将不再连接它。要撤销它对这台手机的授权，请在那台设备的“设备”设置中操作。") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    scope.launch { runCatching { hub.forget(contact.peerId) } }
                }) { Text("移除") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("取消") } },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun DeviceRow(contact: BindingDeviceContact, status: String, open: () -> Unit, remove: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val confirmed = contact.confirmation is BindingContactConfirmation.Confirmed
    Box {
        ListItem(
            headlineContent = { Text(contact.name) },
            supportingContent = { Text(status) },
            modifier = Modifier.combinedClickable(enabled = true, onClick = { if (confirmed) open() else menu = true }, onLongClick = { menu = true }),
        )
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            if (confirmed) DropdownMenuItem(text = { Text("打开") }, onClick = { menu = false; open() })
            DropdownMenuItem(text = { Text("移除此设备") }, onClick = { menu = false; remove() })
        }
    }
}

fun deviceStatus(state: BindingRemoteServiceSnapshot, contact: BindingDeviceContact, path: String? = null): String {
    state.outgoingPairings.firstOrNull { it.peerId == contact.peerId }?.takeIf { it.phase != BindingPairingPhase.GRANTED }?.let {
        return when (it.phase) {
            BindingPairingPhase.AWAITING_READY, BindingPairingPhase.AWAITING_APPROVAL -> "等待对方允许 · 核对码 ${grouped(it.verificationCode.orEmpty())}"
            BindingPairingPhase.REJECTED -> "对方已拒绝"
            BindingPairingPhase.EXPIRED -> "邀请已过期"
            BindingPairingPhase.DISCONNECTED -> "配对时连接中断"
            BindingPairingPhase.GRANTED -> ""
        }
    }
    if (contact.confirmation is BindingContactConfirmation.Pending) return "等待对方允许"
    val controller = state.controllers.firstOrNull { it.peerId == contact.peerId } ?: return "未连接"
    return when (controller.phase) {
        BindingControllerPhase.CONNECTED -> listOfNotNull("已连接", path).joinToString(" · ")
        BindingControllerPhase.CONNECTING, BindingControllerPhase.DISCOVERING -> "正在连接"
        BindingControllerPhase.RECOVERING -> "正在重新连接"
        BindingControllerPhase.SUSPENDED, BindingControllerPhase.DISCONNECTED -> "未连接"
        BindingControllerPhase.BLOCKED -> "无法连接" + (controller.errorCode?.let { "（$it）" } ?: "")
    }
}

fun grouped(code: String): String = if (code.length == 8) code.take(4) + " " + code.takeLast(4) else code

@Composable
fun Message(text: String) {
    Column(Modifier.fillMaxSize().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(text, style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 96.dp))
    }
}
