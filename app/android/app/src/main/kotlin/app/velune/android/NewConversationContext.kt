package app.velune.android

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

data class NewSessionContext(
    val peer: String,
    val target: ConversationTarget.New,
    val devices: List<Pair<DeviceSummary, DeviceListing>>,
    val selectDevice: (String) -> Unit,
    val selectRuntime: (String) -> Unit,
    val selectProject: (String?) -> Unit,
)

@Composable
fun NewConversationContext(context: NewSessionContext, enabled: Boolean, hasAttachments: Boolean) {
    val (peer, target, devices) = context
    val entry = devices.firstOrNull { it.first.peerId == peer }
    val device = entry?.first
    val listing = entry?.second ?: DeviceListing()
    val availableDevices = devices.filter { it.canStartConversation() }
    val spacing = LocalAppearance.current.spacingScale
    var choosingDevice by remember { mutableStateOf(false) }
    var choosingRuntime by remember { mutableStateOf(false) }
    var choosingProject by remember { mutableStateOf(false) }
    val projects = listing.workingProjects(target.runtimeId)
    val directories = remember(projects) {
        projects.asSequence()
            .flatMap { project -> project.directories.asSequence().map { it to project.name } }
            .distinctBy { it.first }
            .toList()
    }
    val project = target.cwd?.let { cwd -> directories.firstOrNull { it.first == cwd }?.second ?: lastPathComponent(cwd) }
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp * spacing),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1.2f)) {
            ContextButton("设备", device?.name ?: "选择设备", enabled) { choosingDevice = true }
            DropdownMenu(expanded = choosingDevice, onDismissRequest = { choosingDevice = false }) {
                if (availableDevices.isEmpty()) {
                    DropdownMenuItem(text = { Text("没有可用设备") }, enabled = false, onClick = {})
                } else if (hasAttachments && availableDevices.any { it.first.peerId != peer }) {
                    Text(
                        "切换时带上正文，附件留在原设备。",
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                    )
                }
                availableDevices.forEach { (option, _) ->
                    DropdownMenuItem(
                        text = { Text(option.name) },
                        leadingIcon = { RadioButton(selected = option.peerId == peer, onClick = null) },
                        onClick = { choosingDevice = false; context.selectDevice(option.peerId) },
                    )
                }
            }
        }
        ContextDivider()
        Box(Modifier.weight(1f)) {
            ContextButton("运行时", listing.runtimeLabels[target.runtimeId]?.name ?: target.runtimeId, enabled) { choosingRuntime = true }
            DropdownMenu(expanded = choosingRuntime, onDismissRequest = { choosingRuntime = false }) {
                if (listing.runtimes.isEmpty()) {
                    DropdownMenuItem(text = { Text("没有可用运行时") }, enabled = false, onClick = {})
                }
                listing.runtimes.forEach { runtime ->
                    DropdownMenuItem(
                        text = { Text(runtime.name) },
                        leadingIcon = { RadioButton(selected = runtime.id == target.runtimeId, onClick = null) },
                        onClick = { choosingRuntime = false; context.selectRuntime(runtime.id) },
                    )
                }
            }
        }
        ContextDivider()
        Box(Modifier.weight(1f)) {
            ContextButton("项目", project ?: "默认目录", enabled) { choosingProject = true }
        }
    }
    if (choosingProject) ProjectSheet(directories, target.cwd, dismiss = { choosingProject = false }) {
        choosingProject = false
        context.selectProject(it)
    }
}

@Composable
private fun ContextButton(label: String, value: String, enabled: Boolean, onClick: () -> Unit) {
    val color = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = "$label：$value" }
            .padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(value, style = MaterialTheme.typography.labelLarge, color = color, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
        Icon(Icons.Filled.ArrowDropDown, contentDescription = null, modifier = Modifier.size(16.dp), tint = color)
    }
}

@Composable
private fun ContextDivider() {
    Text("·", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ProjectSheet(directories: List<Pair<String, String>>, cwd: String?, dismiss: () -> Unit, select: (String?) -> Unit) {
    var search by rememberSaveable { mutableStateOf("") }
    val query = search.trim()
    val paths = remember(query, directories) {
        directories.filter { (path, name) ->
            path.contains(query, ignoreCase = true) || name.contains(query, ignoreCase = true)
        }
    }
    ModalBottomSheet(sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), onDismissRequest = dismiss) {
        Text("项目", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp))
        OutlinedTextField(
            value = search,
            onValueChange = { search = it },
            label = { Text("搜索项目") },
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp),
        )
        LazyColumn(Modifier.padding(bottom = 12.dp), horizontalAlignment = Alignment.Start) {
            item(key = "default") {
                ProjectChoice("不指定", "运行时默认目录", cwd == null) { select(null) }
            }
            if (paths.isNotEmpty() && query.isEmpty()) {
                item(key = "recent") {
                    Text("最近项目", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(start = 24.dp, top = 12.dp, bottom = 4.dp))
                }
            }
            items(paths, key = { "project/${it.first}" }) { (path, name) ->
                ProjectChoice(name, path, path == cwd) { select(path) }
            }
            if (paths.isEmpty()) {
                item(key = "empty") {
                    Text(if (query.isEmpty()) "没有最近项目" else "没有匹配的项目", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp))
                }
            }
        }
    }
}

@Composable
private fun ProjectChoice(title: String, detail: String, selected: Boolean, select: () -> Unit) {
    ListItem(
        leadingContent = { RadioButton(selected = selected, onClick = null) },
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(detail, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.selectable(selected, role = Role.RadioButton, onClick = select),
    )
}
