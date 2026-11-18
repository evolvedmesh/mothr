package dev.mothr.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mothr.client.ChatStatus
import dev.mothr.client.ChatSummary
import dev.mothr.client.ConnectionState
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_bell
import dev.mothr.mobile.resources.ic_check
import dev.mothr.mobile.resources.ic_layers
import dev.mothr.mobile.resources.ic_log_out
import dev.mothr.mobile.resources.ic_palette
import dev.mothr.mobile.resources.ic_plus
import dev.mothr.mobile.resources.ic_search
import dev.mothr.mobile.resources.ic_settings_2
import dev.mothr.mobile.resources.ic_wifi_off
import dev.mothr.mobile.resources.ic_x

@Composable
fun ChatsScreen(model: AppModel) {
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
        Header(model)
        ConnectionBanner(model)
        SearchField(model)
        val groups = model.groups
        if (groups.isEmpty()) {
            EmptyState(model)
        } else {
            LazyColumn(
                Modifier.fillMaxSize(),
                contentPadding = PaddingValues(bottom = 24.dp + navBottom()),
            ) {
                groups.forEach { g ->
                    item(key = "h-" + g.title) {
                        Text(
                            g.title.uppercase(),
                            style = type.caption.copy(color = if (g.title == "Needs you") pal.warning else pal.textFaint, letterSpacing = androidx.compose.ui.unit.TextUnit(0.6f, androidx.compose.ui.unit.TextUnitType.Sp)),
                            modifier = Modifier.padding(start = 20.dp, top = 18.dp, bottom = 6.dp),
                        )
                    }
                    items(g.chats, key = { g.title + it.id }) { chat -> ChatRow(chat) { model.open(chat.id) } }
                }
            }
        }
    }
}

@Composable
fun navBottom() = androidx.compose.foundation.layout.WindowInsets.navigationBars.let {
    with(androidx.compose.ui.platform.LocalDensity.current) { it.getBottom(this).toDp() }
}

@Composable
private fun Header(model: AppModel) {
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().padding(start = 20.dp, end = 10.dp, top = 10.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text("Chats", style = type.display)
            val conn = model.connection
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                val (color, label) = when (conn?.state) {
                    ConnectionState.CONNECTED -> pal.success to conn.server.removePrefix("https://").removePrefix("http://")
                    ConnectionState.UNAUTHORIZED -> pal.danger to "Not paired"
                    ConnectionState.RECONNECTING -> pal.warning to "Reconnecting…"
                    else -> pal.textFaint to "Connecting…"
                }
                Box(Modifier.size(6.dp).clip(Radius.full).background(color))
                Text(label, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        UsageButton(model)
        Box {
            IconBtn(Res.drawable.ic_settings_2) { menu = true }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = pal.raised) {
                DropdownMenuItem(
                    text = {
                        Column {
                            Text("Run in background", style = type.body)
                            Text("Notifies you when an agent needs you", style = type.caption.copy(color = pal.textFaint))
                        }
                    },
                    leadingIcon = { Ico(Res.drawable.ic_bell) },
                    trailingIcon = { if (model.runInBackground) Ico(Res.drawable.ic_check, tint = pal.brand) },
                    onClick = { model.toggleBackground(!model.runInBackground) },
                )
                DropdownMenuItem(
                    text = { Text("Unpair this phone", style = type.body) },
                    leadingIcon = { Ico(Res.drawable.ic_log_out) },
                    onClick = { menu = false; model.unpair() },
                )
            }
        }
        Spacer(Modifier.width(4.dp))
        IconBtn(Res.drawable.ic_plus, tint = pal.bg, bg = pal.text) { model.showNewChat = true }
    }
}

/** The single usage control: one ring per agent with limits, tap for details. */
@Composable
fun UsageButton(model: AppModel) {
    val usage = model.usage.filter { it.windows.isNotEmpty() }
    if (usage.isEmpty()) return
    Row(
        Modifier.clip(Radius.full).tap(shape = Radius.full) { model.showUsage = true }.padding(horizontal = 8.dp, vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        usage.take(4).forEach { u -> Ring(u.worstFraction, levelColor(u.level, pal), size = 20.dp, stroke = 2.5.dp) }
    }
}

@Composable
private fun ConnectionBanner(model: AppModel) {
    val conn = model.connection ?: return
    val (show, text) = when (conn.state) {
        ConnectionState.RECONNECTING -> true to (conn.message ?: "Lost the connection. Retrying…")
        ConnectionState.UNAUTHORIZED -> true to (conn.message ?: "Pair again")
        else -> false to ""
    }
    if (!show) return
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 6.dp).fillMaxWidth().clip(Radius.md)
            .background(pal.warning.copy(alpha = 0.12f)).padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Ico(Res.drawable.ic_wifi_off, tint = pal.warning, size = 16.dp)
        Text(text, style = type.body.copy(color = pal.text), modifier = Modifier.weight(1f))
        if (conn.state == ConnectionState.UNAUTHORIZED) Btn("Pair", tone = Tone.Warning) { model.unpair() }
    }
}

@Composable
private fun SearchField(model: AppModel) {
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(Radius.md).background(pal.surfaceHover)
            .padding(horizontal = 12.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Ico(Res.drawable.ic_search, tint = pal.textFaint, size = 17.dp)
        Spacer(Modifier.width(10.dp))
        Box(Modifier.weight(1f)) {
            if (model.query.isEmpty()) Text("Search chats", style = type.body.copy(color = pal.textFaint))
            BasicTextField(
                value = model.query,
                onValueChange = { model.search(it) },
                singleLine = true,
                textStyle = type.body,
                cursorBrush = SolidColor(pal.brand),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (model.query.isNotEmpty()) Ico(Res.drawable.ic_x, size = 16.dp, modifier = Modifier.tap { model.search("") })
    }
}

@Composable
fun ChatRow(chat: ChatSummary, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().tap(shape = Radius.md, onClick = onClick).padding(horizontal = 20.dp, vertical = 11.dp),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(13.dp),
    ) {
        AgentAvatar(chat.agent, 38.dp, status = chat.status)
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (chat.design) {
                    Ico(Res.drawable.ic_palette, size = 14.dp)
                    Spacer(Modifier.width(5.dp))
                }
                Text(
                    chat.title,
                    style = type.label.copy(fontSize = androidx.compose.ui.unit.TextUnit(15.5f, androidx.compose.ui.unit.TextUnitType.Sp)),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                Text(chat.updatedLabel, style = type.caption.copy(color = pal.textFaint))
            }
            Spacer(Modifier.height(2.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    buildString {
                        append(chat.projectName)
                        if (chat.preview.isNotEmpty()) append(" · ").append(chat.preview)
                    },
                    style = type.body.copy(color = if (chat.status == ChatStatus.NEEDS_YOU) pal.warning else pal.textMuted, fontSize = androidx.compose.ui.unit.TextUnit(14f, androidx.compose.ui.unit.TextUnitType.Sp)),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                if (chat.unread) {
                    Spacer(Modifier.width(8.dp))
                    Box(Modifier.size(8.dp).clip(Radius.full).background(pal.brand))
                }
            }
            if (chat.subagentsTotal > 0u || chat.queued > 0u) {
                Spacer(Modifier.height(6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (chat.subagentsTotal > 0u) {
                        Pill(
                            "${chat.subagentsRunning}/${chat.subagentsTotal} subagents",
                            color = if (chat.subagentsRunning > 0u) pal.brand else pal.textMuted,
                            bg = if (chat.subagentsRunning > 0u) pal.brandSoft else pal.surfaceHover,
                            icon = Res.drawable.ic_layers,
                        )
                    }
                    if (chat.queued > 0u) Pill("${chat.queued} queued")
                }
            }
        }
    }
}

@Composable
private fun EmptyState(model: AppModel) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val loaded = model.connection?.loaded == true
        Text(if (loaded) (if (model.query.isEmpty()) "No chats yet" else "Nothing matches") else "Loading chats…", style = type.heading)
        Spacer(Modifier.height(6.dp))
        if (loaded && model.query.isEmpty()) {
            Text("Start one here and it runs on your computer.", style = type.body.copy(color = pal.textMuted))
            Spacer(Modifier.height(20.dp))
            Btn("New chat", tone = Tone.Primary, icon = Res.drawable.ic_plus) { model.showNewChat = true }
        }
    }
}
