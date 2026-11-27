package app.velune.android

import android.content.Context
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.compositionLocalOf
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class AppearanceSize(val title: String, val uiScale: Float, val messageScale: Float, val codeScale: Float) {
    Small("小", 0.86f, 0.88f, 0.86f),
    Standard("默认", 0.94f, 0.96f, 0.94f),
    Large("大", 1.04f, 1.08f, 1.06f),
    ExtraLarge("超大", 1.16f, 1.20f, 1.18f),
}

enum class AppearanceDensity(val title: String, val spacingScale: Float) {
    Compact("紧凑", 0.78f),
    Standard("默认", 0.92f),
    Spacious("宽松", 1.12f),
}

@Immutable
data class AppearanceValues(
    val ui: AppearanceSize = AppearanceSize.Standard,
    val messages: AppearanceSize = AppearanceSize.Standard,
    val code: AppearanceSize = AppearanceSize.Standard,
    val density: AppearanceDensity = AppearanceDensity.Standard,
) {
    val uiScale: Float get() = ui.uiScale
    val messageScale: Float get() = messages.messageScale
    val codeScale: Float get() = code.codeScale
    val spacingScale: Float get() = density.spacingScale
}

val LocalAppearance = compositionLocalOf { AppearanceValues() }

class AppearancePreferences(context: Context) {
    private val preferences = context.getSharedPreferences("velune-appearance", Context.MODE_PRIVATE)
    private val current = MutableStateFlow(read())
    val values: StateFlow<AppearanceValues> = current.asStateFlow()

    fun update(transform: (AppearanceValues) -> AppearanceValues) {
        val before = current.value
        val after = transform(before)
        if (before == after) return
        preferences.edit().apply {
            if (after.ui != before.ui) putString("ui-size", after.ui.name)
            if (after.messages != before.messages) putString("message-size", after.messages.name)
            if (after.code != before.code) putString("code-size", after.code.name)
            if (after.density != before.density) putString("density", after.density.name)
        }.apply()
        current.value = after
    }

    fun reset() = update { AppearanceValues() }

    private fun read(): AppearanceValues {
        val saved = preferences.all
        fun size(key: String): AppearanceSize = AppearanceSize.entries.firstOrNull { it.name == saved[key] } ?: AppearanceSize.Standard
        return AppearanceValues(
            ui = size("ui-size"),
            messages = size("message-size"),
            code = size("code-size"),
            density = AppearanceDensity.entries.firstOrNull { it.name == saved["density"] } ?: AppearanceDensity.Standard,
        )
    }
}
