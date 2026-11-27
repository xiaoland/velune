package app.velune.android

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppearanceScreen(preferences: AppearancePreferences, back: () -> Unit) {
    val appearance = LocalAppearance.current
    val spacing = appearance.spacingScale
    Scaffold(topBar = {
        TopAppBar(
            title = { Text("外观与显示") },
            navigationIcon = {
                IconButton(onClick = back) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "返回") }
            },
        )
    }) { padding ->
        Column(
            modifier = Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp * spacing),
            verticalArrangement = Arrangement.spacedBy(12.dp * spacing),
        ) {
            AppearancePreview(appearance)
            AppearanceChoice("界面字号", AppearanceSize.entries, appearance.ui, AppearanceSize::title) { size ->
                preferences.update { it.copy(ui = size) }
            }
            AppearanceChoice("消息字号", AppearanceSize.entries, appearance.messages, AppearanceSize::title) { size ->
                preferences.update { it.copy(messages = size) }
            }
            AppearanceChoice("代码字号", AppearanceSize.entries, appearance.code, AppearanceSize::title) { size ->
                preferences.update { it.copy(code = size) }
            }
            AppearanceChoice("显示密度", AppearanceDensity.entries, appearance.density, AppearanceDensity::title) { density ->
                preferences.update { it.copy(density = density) }
            }
            TextButton(onClick = preferences::reset, enabled = appearance != AppearanceValues()) {
                Text("恢复默认")
            }
        }
    }
}

@Composable
private fun <T> AppearanceChoice(label: String, options: List<T>, selected: T, title: (T) -> String, onSelect: (T) -> Unit) {
    Column {
        Text(label, style = MaterialTheme.typography.titleSmall, modifier = Modifier.semantics { heading() })
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            options.forEachIndexed { index, option ->
                SegmentedButton(
                    selected = option == selected,
                    onClick = { onSelect(option) },
                    shape = SegmentedButtonDefaults.itemShape(index, options.size),
                    contentPadding = PaddingValues(horizontal = 6.dp, vertical = 8.dp),
                    icon = {},
                    label = { Text(title(option)) },
                )
            }
        }
    }
}

@Composable
private fun AppearancePreview(appearance: AppearanceValues) {
    val spacing = appearance.spacingScale
    Card(Modifier.fillMaxWidth()) {
        Column(
            Modifier.padding(16.dp * spacing),
            verticalArrangement = Arrangement.spacedBy(10.dp * spacing),
        ) {
            Text("实时预览", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.semantics { heading() })
            Text("新会话", style = MaterialTheme.typography.titleMedium)
            Text("我会先检查当前项目的构建。", style = BaseTypography.bodyLarge.scaled(appearance.messageScale))
            Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = MaterialTheme.shapes.small, modifier = Modifier.fillMaxWidth()) {
                Text(
                    "git status --short",
                    style = BaseTypography.bodySmall.scaled(appearance.codeScale),
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.padding(10.dp * spacing),
                )
            }
        }
    }
}
