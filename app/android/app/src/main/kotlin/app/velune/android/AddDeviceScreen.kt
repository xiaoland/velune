package app.velune.android

import android.Manifest
import android.content.ClipboardManager
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import app.velune.bindings.BindingContactConfirmation
import app.velune.bindings.BindingPairingOfferInfo
import app.velune.bindings.BindingPairingPhase
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Pairing: read the other device's invitation, compare codes, and wait for it to allow this phone. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AddDeviceScreen(hub: DeviceHub, back: () -> Unit, paired: (String) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by hub.state.collectAsState()
    var camera by remember { mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { camera = it }
    // The invitation's secret stays in memory and is used once.
    var encoded by remember { mutableStateOf<String?>(null) }
    var offer by remember { mutableStateOf<BindingPairingOfferInfo?>(null) }
    var peer by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    fun read(text: String) {
        if (offer != null) return
        try {
            offer = hub.decode(text)
            encoded = text
            failure = null
        } catch (error: Exception) {
            failure = "这不是 Velune 的设备邀请：${error.describe()}"
        }
    }

    val pairing = peer?.let { id -> state?.outgoingPairings?.firstOrNull { it.peerId == id } }
    val confirmed = peer?.let { id -> state?.contacts?.firstOrNull { it.peerId == id }?.confirmation is BindingContactConfirmation.Confirmed } == true
    LaunchedEffect(confirmed) { if (confirmed) paired(peer!!) }
    LaunchedEffect(Unit) { if (!camera) permission.launch(Manifest.permission.CAMERA) }

    Scaffold(topBar = {
        TopAppBar(
            title = { Text("添加设备") },
            navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
        )
    }) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).padding(horizontal = 24.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            val current = offer
            if (current == null) {
                Text("在要控制的 Mac 或 Windows 上打开“设置 › 设备”，选择“添加控制设备”，然后扫描显示的二维码。", style = MaterialTheme.typography.bodyMedium)
                if (camera) {
                    QrScanner(onCode = ::read, modifier = Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(16.dp)))
                } else {
                    OutlinedButton(onClick = { permission.launch(Manifest.permission.CAMERA) }) { Text("允许使用相机") }
                }
                OutlinedButton(onClick = {
                    val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
                    val text = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
                    if (text.isNullOrBlank()) failure = "剪贴板中没有邀请" else read(text)
                }) { Text("粘贴邀请") }
            } else {
                Text("“${current.name}”", style = MaterialTheme.typography.headlineSmall)
                when {
                    pairing == null -> {
                        Text("连接后，两台设备会显示同一组核对码。", style = MaterialTheme.typography.bodyMedium)
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Button(enabled = !busy, onClick = {
                                busy = true
                                scope.launch {
                                    try {
                                        peer = hub.beginPairing(checkNotNull(encoded)).peerId
                                        encoded = null
                                    } catch (error: CancellationException) {
                                        throw error
                                    } catch (error: Exception) {
                                        failure = "无法连接：${error.describe()}"
                                    } finally {
                                        busy = false
                                    }
                                }
                            }) { Text("连接") }
                            OutlinedButton(enabled = !busy, onClick = { offer = null; encoded = null }) { Text("重新扫描") }
                        }
                        if (busy) CircularProgressIndicator()
                    }
                    pairing.phase == BindingPairingPhase.AWAITING_READY || pairing.phase == BindingPairingPhase.AWAITING_APPROVAL || pairing.phase == BindingPairingPhase.GRANTED -> {
                        Text("核对码", style = MaterialTheme.typography.labelLarge)
                        Text(grouped(pairing.verificationCode.orEmpty()), fontFamily = FontFamily.Monospace, fontSize = 40.sp)
                        Text("确认“${current.name}”上显示的核对码与此相同，然后在那台设备上选择“允许”。", style = MaterialTheme.typography.bodyMedium)
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            CircularProgressIndicator(Modifier.padding(4.dp))
                            Text("等待对方允许…")
                        }
                        OutlinedButton(onClick = { scope.launch { runCatching { hub.cancelPairing(pairing.peerId) }; back() } }) { Text("取消") }
                    }
                    else -> {
                        Text(
                            when (pairing.phase) {
                                BindingPairingPhase.REJECTED -> "对方拒绝了这台手机。"
                                BindingPairingPhase.EXPIRED -> "邀请已过期，请在那台设备上重新生成。"
                                else -> "配对时连接中断，请重试。"
                            },
                        )
                        OutlinedButton(onClick = { offer = null; peer = null }) { Text("重新扫描") }
                    }
                }
            }
            failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}
