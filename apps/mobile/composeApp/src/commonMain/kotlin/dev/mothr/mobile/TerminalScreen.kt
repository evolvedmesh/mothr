package dev.mothr.mobile

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.mothr.client.TermSpanView
import dev.mothr.client.TerminalView
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_arrow_left
import dev.mothr.mobile.resources.ic_circle_alert
import dev.mothr.mobile.resources.ic_refresh_cw
import kotlinx.coroutines.delay

/**
 * A chat's terminal: the shell runs on the desktop, in the chat's repo; the phone shows its live
 * screen and types into it. Leaving the screen keeps the shell running.
 */
@Composable
fun TerminalScreen(model: AppModel) {
    val chatId = model.terminalFor ?: return
    val term = model.terminal
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    var ctrl by remember(chatId) { mutableStateOf(false) }

    // A key from the key row; an armed Ctrl applies to it.
    val press: (String) -> Unit = { data ->
        model.typeInTerminal(if (ctrl) withCtrl(data) else data)
        ctrl = false
    }
    val insets = WindowInsets.statusBars.union(WindowInsets.ime.union(WindowInsets.navigationBars))
    Column(Modifier.fillMaxSize().background(pal.bg).windowInsetsPadding(insets)) {
        TerminalHeader(model, term)
        Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
        term?.exited?.let { ExitedBanner(it) { model.restartShell() } }
        TerminalGrid(
            term,
            Modifier.weight(1f).fillMaxWidth(),
            onTap = {
                focus.requestFocus()
                keyboard?.show()
            },
            onFit = { cols, rows -> model.fitTerminal(cols, rows) },
        )
        Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
        KeyRow(ctrl, onCtrl = { ctrl = !ctrl }, onKey = press)
        TerminalInput(
            focus,
            ctrl = ctrl,
            onCtrlUsed = { ctrl = false },
            onSend = model::typeInTerminal,
        )
    }
    // Typing is what this screen is for: bring the keyboard up.
    LaunchedEffect(chatId) { focus.requestFocus() }
}

@Composable
private fun TerminalHeader(model: AppModel, term: TerminalView?) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, end = 8.dp, top = 4.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconBtn(Res.drawable.ic_arrow_left, tint = pal.text) { model.hideTerminal() }
        Spacer(Modifier.width(6.dp))
        Column(Modifier.weight(1f)) {
            val title = term?.title?.takeIf { it.isNotBlank() }
            Text(title ?: term?.cwd ?: "Terminal", style = type.heading, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val detail = when {
                term == null -> "Starting…"
                term.exited != null -> "Ended"
                else -> listOfNotNull(if (title != null) term.cwd else null, "${term.cols}×${term.rows}").joinToString(" · ")
            }
            Text(detail, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        IconBtn(Res.drawable.ic_refresh_cw) { model.restartShell() }
        Text(
            "End shell",
            style = type.label.copy(color = pal.textMuted),
            modifier = Modifier.tap { model.endShell() }.padding(horizontal = 10.dp, vertical = 10.dp),
        )
    }
}

@Composable
private fun ExitedBanner(reason: String, onRestart: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().background(pal.surface).padding(start = 16.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Ico(Res.drawable.ic_circle_alert, tint = pal.textMuted, size = 16.dp)
        Column(Modifier.weight(1f)) {
            Text("The shell ended", style = type.label)
            if (reason.isNotBlank()) Text(reason, style = type.caption, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Btn("Restart", tone = Tone.Primary, icon = Res.drawable.ic_refresh_cw, onClick = onRestart)
    }
    Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
}

/** The live screen. Its size decides how many cols and rows the shell gets. */
@Composable
private fun TerminalGrid(term: TerminalView?, modifier: Modifier, onTap: () -> Unit, onFit: (Int, Int) -> Unit) {
    val p = pal
    val screenBg = p.codeBg
    val style = type.mono.copy(fontSize = 12.sp, lineHeight = 16.sp, color = p.text)
    val measurer = rememberTextMeasurer()
    // One cell: the advance of a monospace char and the distance between rows.
    val cell = remember(style, measurer) {
        val ten = measurer.measure("MMMMMMMMMM", style, softWrap = false)
        val one = measurer.measure("M", style, softWrap = false)
        val two = measurer.measure("M\nM", style, softWrap = false)
        Pair(ten.size.width / 10f, (two.size.height - one.size.height).toFloat().coerceAtLeast(1f))
    }
    val density = LocalDensity.current
    val padX = 8.dp
    val padY = 6.dp
    BoxWithConstraints(
        modifier.background(screenBg).clickable(remember { MutableInteractionSource() }, indication = null, onClick = onTap),
    ) {
        val (cellW, cellH) = cell
        val padXPx = with(density) { padX.toPx() }
        val padYPx = with(density) { padY.toPx() }
        val cols = ((constraints.maxWidth - 2 * padXPx) / cellW).toInt().coerceIn(20, 500)
        val rows = ((constraints.maxHeight - 2 * padYPx) / cellH).toInt().coerceIn(4, 300)
        val alive = term != null && term.exited == null
        // Fit the shell to the phone; the keyboard coming and going changes the size, so wait
        // for it to settle.
        LaunchedEffect(cols, rows, alive) {
            if (!alive) return@LaunchedEffect
            delay(250)
            onFit(cols, rows)
        }
        if (term == null) {
            Text("Starting shell…", style = type.body.copy(color = p.textMuted), modifier = Modifier.align(Alignment.Center))
            return@BoxWithConstraints
        }
        val text = remember(term, p) { screenText(term, p, alive) }
        val h = rememberScrollState()
        val v = rememberScrollState()
        // A screen bigger than the phone (sized by the desktop): keep the cursor in view.
        val viewW = constraints.maxWidth - 2 * padXPx
        val viewH = constraints.maxHeight - 2 * padYPx
        LaunchedEffect(term.cursorRow, term.cursorCol, term.rows, term.cols) {
            keepInView(v, term.cursorRow?.toInt(), cellH, viewH)
            keepInView(h, term.cursorCol?.toInt(), cellW, viewW)
        }
        Box(Modifier.fillMaxSize().verticalScroll(v).horizontalScroll(h).padding(horizontal = padX, vertical = padY)) {
            Text(text, style = style, softWrap = false)
        }
    }
}

private suspend fun keepInView(scroll: ScrollState, index: Int?, size: Float, view: Float) {
    if (index == null || scroll.maxValue == 0) return
    val start = index * size
    val end = start + size
    when {
        end > scroll.value + view -> scroll.scrollTo((end - view).toInt())
        start < scroll.value -> scroll.scrollTo(start.toInt())
    }
}

/** The key row: what a phone keyboard lacks. */
@Composable
private fun KeyRow(ctrl: Boolean, onCtrl: () -> Unit, onKey: (String) -> Unit) {
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        KeyCap("Esc") { onKey(ESC) }
        KeyCap("Tab") { onKey("\t") }
        KeyCap("Ctrl", active = ctrl, onClick = onCtrl)
        KeyCap("↑") { onKey("$ESC[A") }
        KeyCap("↓") { onKey("$ESC[B") }
        KeyCap("←") { onKey("$ESC[D") }
        KeyCap("→") { onKey("$ESC[C") }
        KeyCap("^C") { onKey("\u0003") }
        KeyCap("|") { onKey("|") }
        KeyCap("~") { onKey("~") }
        KeyCap("/") { onKey("/") }
        KeyCap("-") { onKey("-") }
    }
}

@Composable
private fun KeyCap(label: String, active: Boolean = false, onClick: () -> Unit) {
    val p = pal
    Box(
        Modifier.heightIn(min = 36.dp).widthIn(min = 40.dp).clip(Radius.sm)
            .background(if (active) p.brandSoft else p.surfaceHover)
            .border(1.dp, if (active) p.brand else Color.Transparent, Radius.sm)
            .tap(shape = Radius.sm, onClick = onClick)
            .padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, style = type.mono.copy(fontSize = 14.sp, color = if (active) p.brand else p.text, fontWeight = FontWeight.Medium))
    }
}

/**
 * The field the keyboard types into. Each change is sent as it happens (the shell echoes it on
 * the screen above); the field only keeps the current line so the keyboard can compose words.
 * It always starts with an invisible marker, so Backspace on an "empty" line still reaches the
 * shell.
 */
@Composable
private fun TerminalInput(focus: FocusRequester, ctrl: Boolean, onCtrlUsed: () -> Unit, onSend: (String) -> Unit) {
    var value by remember { mutableStateOf(TextFieldValue(MARK, TextRange(MARK.length))) }
    val style = type.mono.copy(fontSize = 15.sp, lineHeight = 21.sp, color = pal.text)
    Box(
        Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, bottom = 8.dp).clip(Radius.md)
            .background(pal.surface).border(1.dp, pal.borderStrong, Radius.md)
            .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        if (value.text.length <= MARK.length) Text("Type to the shell", style = style.copy(color = pal.textFaint))
        BasicTextField(
            value = value,
            onValueChange = { next ->
                val old = value.text
                val new = next.text
                if (new == old) {
                    value = next
                    return@BasicTextField
                }
                val kept = new.startsWith(MARK)
                var data = typedChange(old.removePrefix(MARK), new.removePrefix(MARK))
                // The marker itself went: one more Backspace.
                if (!kept && new.isEmpty()) data += DEL
                data = data.replace("\r\n", "\r").replace('\n', '\r')
                var reset = !kept || '\n' in new || '\r' in new || (next.composition == null && new.length > 120)
                if (ctrl && data.isNotEmpty() && !data.startsWith(DEL)) {
                    data = withCtrl(data.take(1)) + data.drop(1)
                    onCtrlUsed()
                    reset = true
                }
                if (data.isNotEmpty()) onSend(data)
                value = if (reset) TextFieldValue(MARK, TextRange(MARK.length)) else next
            },
            textStyle = style,
            cursorBrush = SolidColor(pal.brand),
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Ascii,
            ),
            maxLines = 3,
            modifier = Modifier.fillMaxWidth().focusRequester(focus).onPreviewKeyEvent { e ->
                // A hardware keyboard's keys that the field would otherwise keep for itself.
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                val seq = when (e.key) {
                    Key.DirectionUp -> "$ESC[A"
                    Key.DirectionDown -> "$ESC[B"
                    Key.DirectionRight -> "$ESC[C"
                    Key.DirectionLeft -> "$ESC[D"
                    Key.Escape -> ESC
                    Key.Tab -> "\t"
                    else -> null
                } ?: return@onPreviewKeyEvent false
                onSend(seq)
                true
            },
        )
    }
}

private const val ESC = "\u001b"
private const val DEL = "\u007f"

/** Zero-width marker at the start of the input field (see [TerminalInput]). */
private const val MARK = "​"

/** What to send to turn [old] into [new]: a Backspace per removed char, then what was added. */
internal fun typedChange(old: String, new: String): String {
    val common = old.commonPrefixWith(new).length
    val removed = codePoints(old.substring(common))
    return DEL.repeat(removed) + new.substring(common)
}

private fun codePoints(s: String): Int {
    var n = 0
    var i = 0
    while (i < s.length) {
        i += if (s[i].isHighSurrogate() && i + 1 < s.length && s[i + 1].isLowSurrogate()) 2 else 1
        n++
    }
    return n
}

/** A key with Ctrl held: a letter becomes its control code; arrows get the Ctrl modifier. */
internal fun withCtrl(data: String): String {
    if (data.length == 3 && data.startsWith("$ESC[") && data[2] in "ABCD") return "$ESC[1;5${data[2]}"
    if (data.length != 1) return data
    val c = data[0]
    return when (c) {
        in 'a'..'z', in 'A'..'Z', in '@'..'_' -> (c.uppercaseChar().code and 0x1f).toChar().toString()
        ' ' -> "\u0000"
        '?' -> DEL
        else -> data
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering

/** The 16 theme colors (black, red, green, yellow, blue, magenta, cyan, white, then bright). */
private val AnsiDark = longArrayOf(
    0x2B2C35, 0xF2555A, 0x3DD68C, 0xF5B544, 0x5B9DFF, 0x8B7CF6, 0x2DD4BF, 0xC9CBD6,
    0x60626E, 0xFF7A7E, 0x6EE7A8, 0xFFD07A, 0x8AB8FF, 0xB0A5FA, 0x5EEAD4, 0xECECF1,
)
private val AnsiLight = longArrayOf(
    0x2A2B33, 0xDC3B40, 0x14A363, 0xC7860A, 0x2F6FE0, 0x6D5BEF, 0x0E9384, 0x9698A4,
    0x5F6170, 0xE5484D, 0x18B871, 0xB57A08, 0x3B7BF0, 0x7C6CF2, 0x0FA697, 0xC2C3CC,
)

/** A terminal color: 0–255 from the xterm table, or 0xRRGGBB with bit 24 set. */
internal fun termColor(value: UInt, dark: Boolean): Color {
    if (value >= 0x1000000u) return rgb((value and 0xFFFFFFu).toLong())
    val i = value.toInt()
    return when {
        i < 16 -> rgb((if (dark) AnsiDark else AnsiLight)[i])
        i < 232 -> {
            val n = i - 16
            fun level(x: Int) = if (x == 0) 0 else 55 + 40 * x
            rgb(((level(n / 36) shl 16) or (level(n / 6 % 6) shl 8) or level(n % 6)).toLong())
        }
        i < 256 -> {
            val g = 8 + 10 * (i - 232)
            rgb(((g shl 16) or (g shl 8) or g).toLong())
        }
        else -> Color.Unspecified
    }
}

private fun rgb(hex: Long) = Color(0xFF000000 or hex)

private fun spanStyle(s: TermSpanView, p: Palette): SpanStyle {
    var fg = s.fg?.let { termColor(it, p.dark) } ?: p.text
    var bg = s.bg?.let { termColor(it, p.dark) }
    if (s.inverse) {
        val f = fg
        fg = bg ?: p.codeBg
        bg = f
    }
    if (s.dim) fg = fg.copy(alpha = fg.alpha * 0.6f)
    return SpanStyle(
        color = fg,
        background = bg ?: Color.Unspecified,
        fontWeight = if (s.bold) FontWeight.Bold else null,
        fontStyle = if (s.italic) FontStyle.Italic else null,
        textDecoration = if (s.underline) TextDecoration.Underline else null,
    )
}

/** The whole screen as one string, a line per row, with the cursor cell drawn in. */
private fun screenText(term: TerminalView, p: Palette, showCursor: Boolean): AnnotatedString = buildAnnotatedString {
    val cursor = SpanStyle(color = p.onBrand, background = p.brand)
    val rows = maxOf(term.rows.toInt(), term.lines.size)
    val cursorRow = term.cursorRow?.toInt()
    for (r in 0 until rows) {
        if (r > 0) append('\n')
        val cursorCol = if (showCursor && r == cursorRow) term.cursorCol?.toInt() else null
        var drawn = cursorCol == null
        var col = 0
        for (s in term.lines.getOrNull(r)?.spans.orEmpty()) {
            val style = spanStyle(s, p)
            val at = if (drawn) null else charAtCell(s.text, cursorCol!! - col)
            if (at != null) {
                withStyle(style) { append(s.text.substring(0, at.first)) }
                withStyle(cursor) { append(s.text.substring(at.first, at.last + 1)) }
                withStyle(style) { append(s.text.substring(at.last + 1)) }
                drawn = true
            } else {
                withStyle(style) { append(s.text) }
            }
            col += cells(s.text)
        }
        // The cursor past the end of the line (trailing blanks are left out).
        if (!drawn && cursorCol != null) {
            append(" ".repeat((cursorCol - col).coerceAtLeast(0)))
            withStyle(cursor) { append(' ') }
        }
    }
}

/** The char range (start..end index) covering terminal cell [cell] of [text], or null if past it. */
private fun charAtCell(text: String, cell: Int): IntRange? {
    if (cell < 0) return null
    var col = 0
    var i = 0
    while (i < text.length) {
        val (cp, len) = codePointAt(text, i)
        val w = cellWidth(cp)
        if (cell < col + w) return i until i + len
        col += w
        i += len
    }
    return null
}

private fun cells(text: String): Int {
    var col = 0
    var i = 0
    while (i < text.length) {
        val (cp, len) = codePointAt(text, i)
        col += cellWidth(cp)
        i += len
    }
    return col
}

private fun codePointAt(s: String, i: Int): Pair<Int, Int> {
    val c = s[i]
    if (c.isHighSurrogate() && i + 1 < s.length && s[i + 1].isLowSurrogate()) {
        return Pair(((c.code - 0xD800) shl 10) + (s[i + 1].code - 0xDC00) + 0x10000, 2)
    }
    return Pair(c.code, 1)
}

/** Wide (two-cell) chars: CJK, Hangul, fullwidth forms and most emoji. */
private fun cellWidth(cp: Int): Int = when (cp) {
    in 0x1100..0x115F, in 0x2E80..0x303E, in 0x3041..0x33FF, in 0x3400..0x4DBF, in 0x4E00..0x9FFF,
    in 0xA000..0xA4CF, in 0xAC00..0xD7A3, in 0xF900..0xFAFF, in 0xFE30..0xFE4F, in 0xFF00..0xFF60,
    in 0xFFE0..0xFFE6, in 0x1F300..0x1F64F, in 0x1F900..0x1F9FF, in 0x20000..0x3FFFD -> 2
    else -> 1
}
