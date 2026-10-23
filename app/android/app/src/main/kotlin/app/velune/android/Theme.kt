package app.velune.android

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

/**
 * Material 3 with the system's dynamic colours where Android has them (12+).
 * Older phones get a scheme seeded from the brand's slate blue (#7686B5)
 * instead of Material's baseline purple.
 */
@Composable
fun VeluneTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colors = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> BrandDark
        else -> BrandLight
    }
    MaterialTheme(colorScheme = colors, content = content)
}

private val BrandLight: ColorScheme = lightColorScheme(
    primary = Color(0xFF4C5C92), onPrimary = Color.White,
    primaryContainer = Color(0xFFDCE1FF), onPrimaryContainer = Color(0xFF041749),
    secondary = Color(0xFF595D72), onSecondary = Color.White,
    secondaryContainer = Color(0xFFDEE1F9), onSecondaryContainer = Color(0xFF161B2C),
    tertiary = Color(0xFF75546F), onTertiary = Color.White,
    tertiaryContainer = Color(0xFFFFD7F5), onTertiaryContainer = Color(0xFF2C1229),
    background = Color(0xFFFAF8FF), onBackground = Color(0xFF1A1B21),
    surface = Color(0xFFFAF8FF), onSurface = Color(0xFF1A1B21),
    surfaceVariant = Color(0xFFE2E1EC), onSurfaceVariant = Color(0xFF45464F),
    outline = Color(0xFF767680), outlineVariant = Color(0xFFC6C5D0),
    surfaceContainerLowest = Color.White, surfaceContainerLow = Color(0xFFF4F3FA),
    surfaceContainer = Color(0xFFEEEDF4), surfaceContainerHigh = Color(0xFFE8E7EF),
    surfaceContainerHighest = Color(0xFFE3E1E9),
)

private val BrandDark: ColorScheme = darkColorScheme(
    primary = Color(0xFFB5C4FF), onPrimary = Color(0xFF1C2D61),
    primaryContainer = Color(0xFF344479), onPrimaryContainer = Color(0xFFDCE1FF),
    secondary = Color(0xFFC2C5DD), onSecondary = Color(0xFF2B3042),
    secondaryContainer = Color(0xFF424659), onSecondaryContainer = Color(0xFFDEE1F9),
    tertiary = Color(0xFFE3BADA), onTertiary = Color(0xFF43273F),
    tertiaryContainer = Color(0xFF5B3D57), onTertiaryContainer = Color(0xFFFFD7F5),
    background = Color(0xFF121318), onBackground = Color(0xFFE3E1E9),
    surface = Color(0xFF121318), onSurface = Color(0xFFE3E1E9),
    surfaceVariant = Color(0xFF45464F), onSurfaceVariant = Color(0xFFC6C5D0),
    outline = Color(0xFF90909A), outlineVariant = Color(0xFF45464F),
    surfaceContainerLowest = Color(0xFF0D0E13), surfaceContainerLow = Color(0xFF1A1B21),
    surfaceContainer = Color(0xFF1E1F25), surfaceContainerHigh = Color(0xFF292A2F),
    surfaceContainerHighest = Color(0xFF34343A),
)

/** How a status reads at a glance; the text beside it says what it is. */
enum class Tone { Positive, Active, Attention, Neutral, Negative }

/** Material has no green role; connected and done read green on every other client of this kind. */
@Composable
@ReadOnlyComposable
fun Tone.color(): Color = when (this) {
    Tone.Positive -> if (isSystemInDarkTheme()) Color(0xFF6DD58C) else Color(0xFF1E8E3E)
    Tone.Active -> MaterialTheme.colorScheme.primary
    Tone.Attention -> if (isSystemInDarkTheme()) Color(0xFFFFB95C) else Color(0xFFB06000)
    Tone.Neutral -> MaterialTheme.colorScheme.outline
    Tone.Negative -> MaterialTheme.colorScheme.error
}
