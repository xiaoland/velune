package app.velune.android

import android.content.ClipData
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
fun StatusDot(tone: Tone, modifier: Modifier = Modifier, size: Dp = 8.dp) {
    Box(modifier.size(size).background(tone.color(), CircleShape))
}

/** A runtime instance as the lists show it: its own name, and the Harness family behind it. */
data class RuntimeLabel(val name: String, val family: String?)

/**
 * A small monogram for the Harness family, so a list of mixed runtimes can be
 * scanned by agent. Lettered rather than vendor logos, which are trademarks.
 */
@Composable
fun RuntimeBadge(label: RuntimeLabel?, modifier: Modifier = Modifier) {
    val (text, color) = when (label?.family) {
        "claude-code" -> "CC" to Color(0xFFD97757)
        "codex" -> "Cx" to Color(0xFF3C3C43)
        "pi" -> "Pi" to Color(0xFF6E56CF)
        "deepseek-harness" -> "DS" to Color(0xFF4D6BFE)
        "acp" -> "A" to Color(0xFF00897B)
        else -> (label?.name?.take(1)?.uppercase() ?: "?") to MaterialTheme.colorScheme.secondary
    }
    Surface(color = color, contentColor = Color.White, shape = RoundedCornerShape(5.dp), modifier = modifier.size(18.dp)) {
        Box(contentAlignment = Alignment.Center) {
            Text(text, fontSize = 9.sp, fontWeight = FontWeight.Bold, lineHeight = 9.sp, maxLines = 1)
        }
    }
}

/** A small rounded label, such as “等待回应” on a list row. */
@Composable
fun Pill(text: String, container: Color, content: Color, modifier: Modifier = Modifier) {
    Surface(color = container, contentColor = content, shape = RoundedCornerShape(6.dp), modifier = modifier) {
        Text(text, style = MaterialTheme.typography.labelSmall, maxLines = 1, modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp))
    }
}

/** An empty or failed screen: what it is, and what to do next. Scrolls so pull-to-refresh still works. */
@Composable
fun EmptyState(icon: ImageVector, title: String, body: String? = null, action: (@Composable () -> Unit)? = null) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 32.dp, vertical = 72.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Surface(color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer, shape = CircleShape, modifier = Modifier.size(72.dp)) {
            Box(contentAlignment = Alignment.Center) { Icon(icon, contentDescription = null, modifier = Modifier.size(32.dp)) }
        }
        Spacer(Modifier.height(4.dp))
        Text(title, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        body?.let {
            Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.widthIn(max = 360.dp))
        }
        action?.let { Spacer(Modifier.height(4.dp)); it() }
    }
}

fun relativeTime(unixMs: Long): String =
    DateUtils.getRelativeTimeSpanString(unixMs, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE).toString()

/** “8 秒”, “3 分 12 秒”, “1 小时 4 分”. */
fun duration(ms: ULong): String {
    val seconds = (ms / 1_000uL).toLong()
    return when {
        seconds < 60 -> "$seconds 秒"
        seconds < 3_600 -> "${seconds / 60} 分 ${seconds % 60} 秒"
        else -> "${seconds / 3_600} 小时 ${seconds % 3_600 / 60} 分"
    }
}

/** The last component of a POSIX or Windows path. */
fun lastPathComponent(path: String): String = path.trimEnd('/', '\\').substringAfterLast('/').substringAfterLast('\\').ifEmpty { path }

/** Puts text on the clipboard; Android shows its own confirmation from 13 on. */
@Composable
fun rememberCopy(): (String) -> Unit {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    return { text -> scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("Velune", text))) } }
}
