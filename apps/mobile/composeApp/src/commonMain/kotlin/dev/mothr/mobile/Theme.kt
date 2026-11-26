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
import androidx.compose.ui.graphics.luminance
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
    /** Text and icons drawn on the accent. */
    val onBrand: Color = Color.White,
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

/** Light, dark, or whatever the phone uses. */
enum class ThemeMode(val id: String, val label: String) {
    System("system", "System"), Light("light", "Light"), Dark("dark", "Dark");

    companion object { fun of(id: String) = entries.firstOrNull { it.id == id } ?: System }
}

/** The accent presets, the same as the desktop's (mothr_core::Accent). */
enum class AccentPreset(val id: String, val label: String, val dark: Long, val light: Long) {
    Violet("violet", "Violet", 0x8B7CF6, 0x6D5BEF),
    Blue("blue", "Blue", 0x5B9DFF, 0x2F6FE0),
    Teal("teal", "Teal", 0x2DD4BF, 0x0E9384),
    Green("green", "Green", 0x4ADE80, 0x16A34A),
    Orange("orange", "Orange", 0xFB923C, 0xEA580C),
    Rose("rose", "Rose", 0xFB7185, 0xE11D48),
    Pink("pink", "Pink", 0xF472B6, 0xDB2777),
    Mono("mono", "Mono", 0xECECF1, 0x15161A);

    companion object { fun of(id: String) = entries.firstOrNull { it.id == id } ?: Violet }
}

/**
 * Surface tones, the same as the desktop's (mothr_core::Tone): bg, chrome, surface,
 * surfaceHover, raised, border, borderStrong, codeBg for dark and light (null keeps the base).
 */
enum class TonePreset(val id: String, val label: String, val dark: LongArray?, val light: LongArray?) {
    Graphite("graphite", "Graphite", null, null),
    Slate(
        "slate", "Slate",
        longArrayOf(0x0A0D13, 0x0D1119, 0x121722, 0x19202C, 0x151B26, 0x1D2431, 0x293243, 0x0E121A),
        longArrayOf(0xFAFBFD, 0xF1F4F9, 0xFFFFFF, 0xEBEFF6, 0xFFFFFF, 0xE1E6EF, 0xD0D8E4, 0xF3F6FA),
    ),
    Warm(
        "warm", "Warm",
        longArrayOf(0x0F0D0B, 0x13110E, 0x191613, 0x201C18, 0x1C1915, 0x28231E, 0x36302A, 0x14120F),
        longArrayOf(0xFCFBF9, 0xF6F3EF, 0xFFFFFF, 0xF1EDE7, 0xFFFFFF, 0xE8E2DA, 0xD9D1C6, 0xF7F4EF),
    ),
    Black("black", "Black", longArrayOf(0x000000, 0x050506, 0x0C0C0E, 0x141417, 0x111113, 0x1A1A1E, 0x27272C, 0x08080A), null);

    companion object { fun of(id: String) = entries.firstOrNull { it.id == id } ?: Graphite }
}

/** The base palette with a tone and an accent applied. */
fun themedPalette(dark: Boolean, accent: AccentPreset, tone: TonePreset): Palette {
    var p = if (dark) DarkPalette else LightPalette
    (if (dark) tone.dark else tone.light)?.let { t ->
        p = p.copy(
            bg = c(t[0]), chrome = c(t[1]), surface = c(t[2]), surfaceHover = c(t[3]),
            raised = c(t[4]), border = c(t[5]), borderStrong = c(t[6]), codeBg = c(t[7]),
        )
    }
    val hex = if (dark) accent.dark else accent.light
    val brand = c(hex)
    return p.copy(
        brand = brand,
        brandSoft = brand.copy(alpha = if (dark) 0.14f else 0.10f),
        onBrand = if (brand.luminance() > 0.5f) c(0x111216) else Color.White,
    )
}

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
fun MothrTheme(
    mode: ThemeMode = ThemeMode.System,
    accent: AccentPreset = AccentPreset.Violet,
    tone: TonePreset = TonePreset.Graphite,
    content: @Composable () -> Unit,
) {
    val dark = when (mode) {
        ThemeMode.System -> isSystemInDarkTheme()
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
    }
    val pal = themedPalette(dark, accent, tone)
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
            primary = pal.brand, onPrimary = pal.onBrand, background = pal.bg, onBackground = pal.text,
            surface = pal.surface, onSurface = pal.text, surfaceVariant = pal.surfaceHover, onSurfaceVariant = pal.textMuted,
            surfaceContainerLow = pal.surface, surfaceContainer = pal.raised, surfaceContainerHigh = pal.raised,
            outline = pal.borderStrong, outlineVariant = pal.border, error = pal.danger, scrim = Color.Black.copy(alpha = 0.55f),
        )
    } else {
        lightColorScheme(
            primary = pal.brand, onPrimary = pal.onBrand, background = pal.bg, onBackground = pal.text,
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
