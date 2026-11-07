package app.velune.android

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** A bundled legal text: NOTICE.md renders as Markdown, licence texts as plain monospace. */
data class LegalDocument(val asset: String, val title: String)

private val documents = listOf(
    LegalDocument("NOTICE.md", "声明与第三方组件"),
    LegalDocument("LICENSE", "Velune AGPL-3.0-only 许可证"),
    LegalDocument("rust-licenses.txt", "Rust 组件许可"),
    LegalDocument("Apache-2.0.txt", "Apache License 2.0"),
    LegalDocument("MPL-2.0.txt", "Mozilla Public License 2.0"),
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AboutScreen(back: () -> Unit, open: (LegalDocument) -> Unit) {
    val context = LocalContext.current
    val uriHandler = LocalUriHandler.current
    val version = remember { context.packageManager.getPackageInfo(context.packageName, 0).versionName }
    Scaffold(topBar = {
        TopAppBar(
            title = { Text("关于") },
            navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
        )
    }) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Column(Modifier.fillMaxWidth().padding(vertical = 16.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Box(Modifier.size(72.dp).background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(18.dp)), contentAlignment = Alignment.Center) {
                    Image(painterResource(R.drawable.ic_launcher_foreground), contentDescription = null, modifier = Modifier.size(72.dp))
                }
                Text("Velune", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(top = 8.dp))
                Text("版本 ${version.orEmpty()}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(
                    "对应源码归档",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.clickable {
                        val tag = "v${version.orEmpty()}"
                        uriHandler.openUri("https://github.com/xiaoland/velune/releases/download/$tag/velune-$tag-source.tar.gz")
                    },
                )
            }
            Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(16.dp)) {
                Text(
                    "这台手机只控制你的 Mac 或 Windows：Agent 与会话都在那些设备上运行。\n\n界面布局参考了同类开源移动客户端 HAPI、Lody 与 Happy，代码为 Velune 独立编写；详见“声明与第三方组件”。",
                    style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(16.dp),
                )
            }
            Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(16.dp)) {
                Column {
                    documents.forEach { document ->
                        ListItem(
                            headlineContent = { Text(document.title) },
                            trailingContent = { Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null) },
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            modifier = Modifier.clickable { open(document) },
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LegalDocumentScreen(document: LegalDocument, back: () -> Unit) {
    val context = LocalContext.current
    var text by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(document) {
        text = withContext(Dispatchers.IO) { context.assets.open(document.asset).bufferedReader().use { it.readText() } }
    }
    Scaffold(topBar = {
        TopAppBar(
            title = { Text(document.title) },
            navigationIcon = { IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") } },
        )
    }) { padding ->
        val loaded = text
        when {
            loaded == null -> Box(Modifier.fillMaxSize().padding(padding)) { CircularProgressIndicator(Modifier.align(Alignment.Center)) }
            document.asset.endsWith(".md") -> SelectionContainer {
                Markdown(loaded, Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp), style = MaterialTheme.typography.bodyMedium)
            }
            // Thousands of lines: one lazy item per paragraph keeps it scrollable.
            else -> {
                val paragraphs = remember(loaded) { loaded.split("\n\n") }
                LazyColumn(Modifier.fillMaxSize().padding(padding), contentPadding = PaddingValues(16.dp)) {
                    itemsIndexed(paragraphs) { _, paragraph ->
                        Text(paragraph + "\n", style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
                    }
                }
            }
        }
    }
}
