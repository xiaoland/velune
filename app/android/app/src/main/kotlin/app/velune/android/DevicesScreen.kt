package app.velune.android

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.velune.bindings.BindingContactConfirmation
import app.velune.bindings.BindingControllerPhase
import app.velune.bindings.BindingDeviceContact
import app.velune.bindings.BindingPairingPhase
import app.velune.bindings.BindingRemoteServiceSnapshot
import kotlinx.coroutines.launch

/** A paired device as the lists show it. */
data class DeviceSummary(val peerId: String, val name: String, val status: DeviceStatus, val confirmed: Boolean)

data class DeviceStatus(val text: String, val tone: Tone)

class DevicesActions(val open: (String) -> Unit, val add: () -> Unit, val remove: (DeviceSummary) -> Unit, val about: () -> Unit)

@Composable
fun DevicesScreen(hub: DeviceHub, open: (String) -> Unit, add: () -> Unit, about: () -> Unit) {
    val state by hub.state.collectAsState()
    val failure by hub.failure.collectAsState()
    val scope = rememberCoroutineScope()
    var removing by remember { mutableStateOf<DeviceSummary?>(null) }
    val devices = state?.let { snapshot -> snapshot.contacts.map { deviceSummary(snapshot, it) } }
    DevicesContent(devices, failure, DevicesActions(open, add, remove = { removing = it }, about))
    removing?.let { device ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("移除“${device.name}”？") },
            text = { Text("这台手机将不再连接它。要撤销它对这台手机的授权，请在那台设备的“设备”设置中操作。") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    scope.launch { runCatching { hub.forget(device.peerId) } }
                }) { Text("移除") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("取消") } },
        )
    }
}

/** `devices` is null while the remote service starts. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesContent(devices: List<DeviceSummary>?, failure: String?, actions: DevicesActions) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("设备") },
                actions = { IconButton(onClick = actions.about) { Icon(Icons.Outlined.Info, contentDescription = "关于与开源许可") } },
            )
        },
        floatingActionButton = {
            if (!devices.isNullOrEmpty()) ExtendedFloatingActionButton(
                onClick = actions.add,
                icon = { Icon(Icons.Filled.Add, contentDescription = null) },
                text = { Text("添加设备") },
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                failure != null -> EmptyState(Icons.Outlined.ErrorOutline, "远程控制没有启动", failure)
                devices == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                devices.isEmpty() -> EmptyState(
                    Icons.Outlined.Devices, "连接你的电脑",
                    "Agent 在 Mac 或 Windows 上运行，这台手机用来查看和控制它们。\n在电脑上打开“设置 › 设备”，选择“添加控制设备”，再用这台手机扫描二维码。",
                ) {
                    Button(onClick = actions.add) {
                        Icon(Icons.Outlined.QrCodeScanner, contentDescription = null, modifier = Modifier.size(18.dp))
                        Text("扫描二维码", modifier = Modifier.padding(start = 8.dp))
                    }
                }
                else -> LazyColumn(
                    Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 96.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(devices, key = { it.peerId }) { device -> DeviceCard(device, actions) }
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun DeviceCard(device: DeviceSummary, actions: DevicesActions) {
    var menu by remember { mutableStateOf(false) }
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(16.dp), modifier = Modifier.fillMaxWidth()) {
        ListItem(
            leadingContent = {
                Surface(color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer, shape = CircleShape, modifier = Modifier.size(44.dp)) {
                    Box(contentAlignment = Alignment.Center) { Icon(Icons.Outlined.Computer, contentDescription = null) }
                }
            },
            headlineContent = { Text(device.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    StatusDot(device.status.tone)
                    Text(device.status.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp))
                }
            },
            trailingContent = {
                Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "更多") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        if (device.confirmed) DropdownMenuItem(text = { Text("打开") }, onClick = { menu = false; actions.open(device.peerId) })
                        DropdownMenuItem(text = { Text("移除此设备") }, onClick = { menu = false; actions.remove(device) })
                    }
                }
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            modifier = Modifier.combinedClickable(onClick = { if (device.confirmed) actions.open(device.peerId) else menu = true }, onLongClick = { menu = true }),
        )
    }
}

fun deviceSummary(state: BindingRemoteServiceSnapshot, contact: BindingDeviceContact, path: String? = null) = DeviceSummary(
    contact.peerId, contact.name, deviceStatus(state, contact, path), contact.confirmation is BindingContactConfirmation.Confirmed,
)

fun deviceStatus(state: BindingRemoteServiceSnapshot, contact: BindingDeviceContact, path: String? = null): DeviceStatus {
    state.outgoingPairings.firstOrNull { it.peerId == contact.peerId }?.takeIf { it.phase != BindingPairingPhase.GRANTED }?.let {
        return when (it.phase) {
            BindingPairingPhase.AWAITING_READY, BindingPairingPhase.AWAITING_APPROVAL -> DeviceStatus("等待对方允许 · 核对码 ${grouped(it.verificationCode.orEmpty())}", Tone.Attention)
            BindingPairingPhase.REJECTED -> DeviceStatus("对方已拒绝", Tone.Negative)
            BindingPairingPhase.EXPIRED -> DeviceStatus("邀请已过期", Tone.Negative)
            BindingPairingPhase.DISCONNECTED -> DeviceStatus("配对时连接中断", Tone.Negative)
            BindingPairingPhase.GRANTED -> DeviceStatus("", Tone.Neutral)
        }
    }
    if (contact.confirmation is BindingContactConfirmation.Pending) return DeviceStatus("等待对方允许", Tone.Attention)
    val controller = state.controllers.firstOrNull { it.peerId == contact.peerId } ?: return DeviceStatus("未连接", Tone.Neutral)
    return when (controller.phase) {
        BindingControllerPhase.CONNECTED -> DeviceStatus(listOfNotNull("已连接", path).joinToString(" · "), Tone.Positive)
        BindingControllerPhase.CONNECTING, BindingControllerPhase.DISCOVERING -> DeviceStatus("正在连接", Tone.Attention)
        BindingControllerPhase.RECOVERING -> DeviceStatus("正在重新连接", Tone.Attention)
        BindingControllerPhase.SUSPENDED, BindingControllerPhase.DISCONNECTED -> DeviceStatus("未连接", Tone.Neutral)
        BindingControllerPhase.BLOCKED -> DeviceStatus("无法连接" + (controller.errorCode?.let { "（$it）" } ?: ""), Tone.Negative)
    }
}

fun grouped(code: String): String = if (code.length == 8) code.take(4) + " " + code.takeLast(4) else code
