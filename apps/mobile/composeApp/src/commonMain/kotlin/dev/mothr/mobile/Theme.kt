package dev.mothr.mobile

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.geist_medium
import dev.mothr.mobile.resources.geist_mono
import dev.mothr.mobile.resources.geist_regular
import dev.mothr.mobile.resources.geist_semibold
import org.jetbrains.compose.resources.Font

/**
 * Mothr's palette, mirrored from the desktop design tokens (apps/desktop/src/ui/tokens.rs).
 * Quiet chrome, one accent: violet marks live state and selection only.
 */
@Immutable
data class Palette(
    val dark: Boolean,
    val bg: Color,
    val chrome: Color,
    val surface: Color,
    val surfaceHover: Color,
    val raised: Color,
    val border: Color,
    val borderStrong: Color,
    val text: Color,
    val textMuted: Color,
    val textFaint: Color,
    val brand: Color,
    val brandSoft: Color,
    val success: Color,
    val warning: Color,
    val danger: Color,
    val codeBg: Color,
)

private fun c(hex: Long) = Color(0xFF000000 or hex)

val DarkPalette = Palette(
    dark = true,
    bg = c(0x0B0C0F), chrome = c(0x0F1014), surface = c(0x14151A), surfaceHover = c(0x1A1B21),
    raised = c(0x17181E), border = c(0x1F2027), borderStrong = c(0x2B2C35),
    text = c(0xECECF1), textMuted = c(0x9B9DAA), textFaint = c(0x60626E),
    brand = c(0x8B7CF6), brandSoft = c(0x8B7CF6).copy(alpha = 0.14f),
    success = c(0x3DD68C), warning = c(0xF5B544), danger = c(0xF2555A), codeBg = c(0x101116),
)

val LightPalette = Palette(
    dark = false,
    bg = c(0xFCFCFD), chrome = c(0xF5F5F8), surface = c(0xFFFFFF), surfaceHover = c(0xF1F1F5),
    raised = c(0xFFFFFF), border = c(0xE7E7EC), borderStrong = c(0xD6D6DE),
    text = c(0x15161A), textMuted = c(0x5F6170), textFaint = c(0x9698A4),
    brand = c(0x6D5BEF), brandSoft = c(0x6D5BEF).copy(alpha = 0.10f),
    success = c(0x14A363), warning = c(0xC7860A), danger = c(0xDC3B40), codeBg = c(0xF6F6F9),
)

/** Type roles from DESIGN.md, nudged up for phone reading distance. */
@Immutable
data class Type(
    val display: TextStyle,
    /** What you write when starting something: large and easy to read. */
    val prompt: TextStyle,
    val title: TextStyle,
    val heading: TextStyle,
    val reading: TextStyle,
    val body: TextStyle,
    val label: TextStyle,
    val caption: TextStyle,
    val mono: TextStyle,
)

val LocalPalette = staticCompositionLocalOf { DarkPalette }
val LocalType = staticCompositionLocalOf<Type> { error("No type") }

object Radius {
    val sm = RoundedCornerShape(6.dp)
    val md = RoundedCornerShape(10.dp)
    val lg = RoundedCornerShape(14.dp)
    val xl = RoundedCornerShape(20.dp)
    val full = RoundedCornerShape(50)
}

@Composable
fun MothrTheme(content: @Composable () -> Unit) {
    val pal = if (isSystemInDarkTheme()) DarkPalette else LightPalette
    val ui = FontFamily(
        Font(Res.font.geist_regular, FontWeight.Normal),
        Font(Res.font.geist_medium, FontWeight.Medium),
        Font(Res.font.geist_semibold, FontWeight.SemiBold),
    )
    val mono = FontFamily(Font(Res.font.geist_mono, FontWeight.Normal))
    val base = TextStyle(fontFamily = ui, color = pal.text)
    val type = Type(
        display = base.copy(fontSize = 28.sp, fontWeight = FontWeight.SemiBold, lineHeight = 32.sp, letterSpacing = (-0.4).sp),
        prompt = base.copy(fontSize = 21.sp, lineHeight = 30.sp, letterSpacing = (-0.15).sp),
        title = base.copy(fontSize = 18.sp, fontWeight = FontWeight.SemiBold, lineHeight = 24.sp, letterSpacing = (-0.2).sp),
        heading = base.copy(fontSize = 16.sp, fontWeight = FontWeight.SemiBold, lineHeight = 22.sp),
        reading = base.copy(fontSize = 15.5.sp, lineHeight = 24.sp),
        body = base.copy(fontSize = 15.sp, lineHeight = 21.sp),
        label = base.copy(fontSize = 14.sp, fontWeight = FontWeight.Medium, lineHeight = 19.sp),
        caption = base.copy(fontSize = 12.5.sp, fontWeight = FontWeight.Medium, lineHeight = 17.sp, color = pal.textMuted),
        mono = TextStyle(fontFamily = mono, fontSize = 13.sp, lineHeight = 19.sp, color = pal.text),
    )
    val scheme = if (pal.dark) {
        darkColorScheme(
            primary = pal.brand, onPrimary = Color.White, background = pal.bg, onBackground = pal.text,
            surface = pal.surface, onSurface = pal.text, surfaceVariant = pal.surfaceHover, onSurfaceVariant = pal.textMuted,
            surfaceContainerLow = pal.surface, surfaceContainer = pal.raised, surfaceContainerHigh = pal.raised,
            outline = pal.borderStrong, outlineVariant = pal.border, error = pal.danger, scrim = Color.Black.copy(alpha = 0.55f),
        )
    } else {
        lightColorScheme(
            primary = pal.brand, onPrimary = Color.White, background = pal.bg, onBackground = pal.text,
            surface = pal.surface, onSurface = pal.text, surfaceVariant = pal.surfaceHover, onSurfaceVariant = pal.textMuted,
            surfaceContainerLow = pal.surface, surfaceContainer = pal.raised, surfaceContainerHigh = pal.raised,
            outline = pal.borderStrong, outlineVariant = pal.border, error = pal.danger,
        )
    }
    CompositionLocalProvider(LocalPalette provides pal, LocalType provides type) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

val pal: Palette @Composable get() = LocalPalette.current
val type: Type @Composable get() = LocalType.current

/** An agent's brand tint (0xRRGGBB from the client) as a Color. */
fun agentColor(rgb: UInt): Color = Color(0xFF000000.toInt() or rgb.toInt())
