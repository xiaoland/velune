package app.velune.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.lifecycle.lifecycleScope
import app.velune.android.preview.PreviewAppearance
import app.velune.android.preview.PreviewGallery

/** Debug-only entry point. The optional "screen" extra selects a scene from PreviewCatalog. */
class GalleryActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val appearance = PreviewAppearance(this)
        val gallery = PreviewGallery(intent.getStringExtra("screen"), lifecycleScope, appearance)
        setContent {
            val values by appearance.draft.values.collectAsState()
            val page = gallery.current
            BackHandler(enabled = gallery.canGoBack, onBack = gallery::back)
            VeluneTheme(values) {
                Surface(Modifier.fillMaxSize()) {
                    key(page) { page.content(gallery) }
                }
            }
        }
    }
}
