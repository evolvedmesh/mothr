package dev.mothr.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_arrow_left
import dev.mothr.mobile.resources.ic_clipboard_paste
import dev.mothr.mobile.resources.ic_link
import dev.mothr.mobile.resources.ic_scan_line

@Suppress("DEPRECATION") // LocalClipboardManager: the replacement is suspend-only and platform-typed.
@Composable
fun PairScreen(model: AppModel) {
    var link by remember { mutableStateOf("") }
    val clipboard = LocalClipboardManager.current
    val adding = model.addingComputer
    val scan = rememberQrScanner(
        onCode = {
            link = it.trim()
            model.pair(link)
        },
        onError = { model.toast = it },
    )
    Column(
        Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding()
            .verticalScroll(rememberScrollState()).padding(horizontal = 24.dp),
    ) {
        if (adding) {
            Spacer(Modifier.height(12.dp))
            IconBtn(Res.drawable.ic_arrow_left, tint = pal.text) { model.cancelAdding() }
            Spacer(Modifier.height(20.dp))
        } else {
            Spacer(Modifier.height(72.dp))
        }
        Logo(56)
        Spacer(Modifier.height(28.dp))
        Text(if (adding) "Add a computer" else "Pair with Mothr", style = type.display)
        Spacer(Modifier.height(10.dp))
        Text(
            (if (adding) "Your other computers stay connected. " else "Control the chats running on your computer from here. ") +
                "On the desktop, open Settings → Remote, " +
                "turn Remote on, then " + if (scan != null) "scan its QR code or copy the pairing link." else "copy the pairing link.",
            style = type.body.copy(color = pal.textMuted),
        )
        Spacer(Modifier.height(32.dp))
        Text("Pairing link", style = type.caption)
        Spacer(Modifier.height(8.dp))
        Row(
            Modifier.fillMaxWidth().clip(Radius.lg).background(pal.surface).border(1.dp, pal.borderStrong, Radius.lg)
                .padding(start = 14.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Ico(Res.drawable.ic_link, size = 18.dp)
            Spacer(Modifier.size(10.dp))
            Box(Modifier.weight(1f).padding(vertical = 10.dp)) {
                if (link.isEmpty()) Text("https://…/?token=…", style = type.mono.copy(color = pal.textFaint))
                BasicTextField(
                    value = link,
                    onValueChange = { link = it.trim() },
                    singleLine = true,
                    textStyle = type.mono,
                    cursorBrush = SolidColor(pal.brand),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go, autoCorrectEnabled = false),
                    keyboardActions = KeyboardActions(onGo = { model.pair(link) }),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            IconBtn(Res.drawable.ic_clipboard_paste) {
                clipboard.getText()?.text?.trim()?.let { link = it }
            }
        }
        model.pairError?.let {
            Spacer(Modifier.height(10.dp))
            Text(it, style = type.body.copy(color = pal.danger))
        }
        Spacer(Modifier.height(20.dp))
        Btn("Connect", tone = Tone.Primary, enabled = link.isNotBlank(), modifier = Modifier.fillMaxWidth()) { model.pair(link) }
        if (scan != null) {
            Spacer(Modifier.height(10.dp))
            Btn("Scan QR code", icon = Res.drawable.ic_scan_line, modifier = Modifier.fillMaxWidth(), onClick = scan)
        }
        Spacer(Modifier.height(36.dp))
        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Hint("1", "Same Wi-Fi works out of the box: Mothr shows a local address.")
            Hint("2", "Away from home, choose Tailscale in Mothr's remote settings and keep it on on this phone too.")
            Hint("3", "The link carries a secret token. Anyone with it can control your agents.")
        }
        Spacer(Modifier.height(40.dp))
    }
}

@Composable
private fun Hint(n: String, text: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(22.dp).clip(Radius.full).background(pal.surfaceHover), contentAlignment = Alignment.Center) {
            Text(n, style = type.caption)
        }
        Text(text, style = type.body.copy(color = pal.textMuted), modifier = Modifier.weight(1f))
    }
}

/** The flat "M" monogram (monochrome, like the desktop's logo mark). */
@Composable
fun Logo(sizeDp: Int) {
    Box(
        Modifier.size(sizeDp.dp).clip(Radius.lg).background(pal.text),
        contentAlignment = Alignment.Center,
    ) {
        Text("M", style = type.display.copy(color = pal.bg, fontSize = androidx.compose.ui.unit.TextUnit(sizeDp * 0.5f, androidx.compose.ui.unit.TextUnitType.Sp)))
    }
}
