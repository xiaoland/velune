package app.velune.android.preview

import android.content.Context
import android.content.ContextWrapper
import android.content.SharedPreferences
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.velune.android.AppearancePreferences
import app.velune.android.AppearanceScreen
import app.velune.android.ConversationTarget
import app.velune.android.DeviceListing
import app.velune.android.DeviceSummary
import app.velune.android.DevicesActions
import app.velune.android.DevicesContent
import app.velune.android.LocalAppearance
import app.velune.android.SessionsActions
import app.velune.android.SessionsContent
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun GalleryDirectory(gallery: PreviewGallery) {
    val spacing = LocalAppearance.current.spacingScale
    Scaffold(topBar = { TopAppBar(title = { Text("场景目录") }) }) { padding ->
        LazyColumn(
            Modifier.fillMaxSize().padding(padding),
            contentPadding = PaddingValues(bottom = 16.dp * spacing),
        ) {
            PreviewCatalog.scenarios.filterNot { it.id == PreviewCatalog.HOME }.groupBy { it.group }.forEach { (group, scenarios) ->
                item(key = group.name) {
                    Text(
                        group.title,
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp * spacing).semantics { heading() },
                    )
                }
                items(scenarios, key = { it.id }) { scenario ->
                    ListItem(
                        headlineContent = { Text(scenario.title) },
                        supportingContent = { Text(scenario.description) },
                        trailingContent = { Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null) },
                        modifier = Modifier.clickable(role = Role.Button) { gallery.open(scenario) },
                    )
                    HorizontalDivider(Modifier.padding(horizontal = 16.dp))
                }
            }
        }
    }
}

@Composable
internal fun PreviewSessions(
    gallery: PreviewGallery,
    listings: List<Pair<DeviceSummary, DeviceListing>>,
    filterable: Boolean = false,
) {
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    SessionsContent(
        devices = listings.map { it.first },
        listings = listings,
        filter = gallery.deviceFilter.takeIf { filterable },
        failure = null,
        refreshing = refreshing,
        actions = SessionsActions(
            open = { peer, target -> if (target is ConversationTarget.Existing) gallery.openSession(peer, target) },
            refresh = {
                if (!refreshing) {
                    refreshing = true
                    scope.launch {
                        delay(450)
                        refreshing = false
                    }
                }
            },
            filter = { gallery.deviceFilter = it },
            devices = { gallery.open("devices") },
            add = { gallery.open("devices") },
            telemetry = {},
            about = {},
            appearance = { gallery.open("appearance") },
            newSession = { gallery.open("new") },
        ),
    )
}

@Composable
internal fun PreviewDevices(gallery: PreviewGallery) {
    DevicesContent(
        devices = PreviewFixtures.devices,
        failure = null,
        actions = DevicesActions(
            back = gallery::back,
            open = {
                gallery.deviceFilter = it
                gallery.open("sessions")
            },
            add = {},
            remove = {},
        ),
    )
}

internal class PreviewAppearance(context: Context) {
    private val saved = AppearancePreferences(PreviewAppearanceContext(context))
    val draft = AppearancePreferences(PreviewAppearanceContext(context, persist = false))
    val savedValues = saved.values

    fun save() = saved.update { draft.values.value }
    fun restore() = draft.update { saved.values.value }
}

@Composable
internal fun PreviewAppearanceScreen(gallery: PreviewGallery) {
    val appearance = gallery.appearance
    val saved by appearance.savedValues.collectAsState()
    val changed = LocalAppearance.current != saved
    Column(Modifier.fillMaxSize().navigationBarsPadding()) {
        Column(Modifier.weight(1f)) {
            AppearanceScreen(appearance.draft, back = gallery::back)
        }
        HorizontalDivider()
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.End,
        ) {
            TextButton(onClick = appearance::restore, enabled = changed) { Text("恢复已保存") }
            Button(onClick = appearance::save, enabled = changed) { Text("保存预览设置") }
        }
    }
}

private class PreviewAppearanceContext(context: Context, private val persist: Boolean = true) : ContextWrapper(context) {
    override fun getSharedPreferences(name: String, mode: Int): SharedPreferences {
        val preferences = super.getSharedPreferences("gallery-$name", mode)
        if (persist) return preferences
        // AppearancePreferences keeps live values in its own StateFlow. Stage nothing on disk until Save.
        return object : SharedPreferences by preferences {
            override fun edit(): SharedPreferences.Editor = object : SharedPreferences.Editor by preferences.edit() {
                override fun putString(key: String?, value: String?): SharedPreferences.Editor = this
                override fun apply() = Unit
                override fun commit() = true
            }
        }
    }
}
