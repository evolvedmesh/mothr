package dev.mothr.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mothr.client.AgentView
import dev.mothr.client.ChoiceOption
import dev.mothr.client.ModeOption
import dev.mothr.client.ModelOption
import dev.mothr.client.ProjectView
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_check
import dev.mothr.mobile.resources.ic_folder
import dev.mothr.mobile.resources.ic_git_branch
import dev.mothr.mobile.resources.ic_globe
import dev.mothr.mobile.resources.ic_log_out
import dev.mothr.mobile.resources.ic_pencil
import dev.mothr.mobile.resources.ic_plus
import dev.mothr.mobile.resources.ic_refresh_cw
import kotlinx.coroutines.delay

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Sheet(onDismiss: () -> Unit, content: @Composable () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = pal.raised,
        scrimColor = androidx.compose.ui.graphics.Color.Black.copy(alpha = if (pal.dark) 0.55f else 0.3f),
        dragHandle = {
            Box(Modifier.padding(top = 10.dp, bottom = 6.dp).size(width = 36.dp, height = 4.dp).clip(Radius.full).background(pal.borderStrong))
        },
    ) { content() }
}

/** Every paired computer: switch between them, name them, add or unpair one. */
@Composable
fun ComputersSheet(model: AppModel) {
    var renaming by remember { mutableStateOf<String?>(null) }
    var unpairing by remember { mutableStateOf<String?>(null) }
    Sheet({ model.showComputers = false }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).imePadding().padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Text("Computers", style = type.title)
            Spacer(Modifier.height(4.dp))
            Text("All of them stay connected, so any of them can notify you.", style = type.body.copy(color = pal.textMuted))
            Spacer(Modifier.height(16.dp))
            Column(Modifier.clip(Radius.lg).border(1.dp, pal.border, Radius.lg)) {
                model.computers.forEach { c ->
                    when (c.id) {
                        renaming -> RenameRow(c, onDone = { name ->
                            model.rename(c, name)
                            renaming = null
                        })
                        unpairing -> Row(
                            Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text("Unpair ${c.name}?", style = type.body, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            Btn("Cancel", tone = Tone.Ghost) { unpairing = null }
                            Btn("Unpair", tone = Tone.Danger) {
                                unpairing = null
                                model.unpair(c)
                            }
                        }
                        else -> ComputerRow(
                            c,
                            selected = c === model.current,
                            onSelect = { model.select(c) },
                            onRename = { unpairing = null; renaming = c.id },
                            onUnpair = { renaming = null; unpairing = c.id },
                        )
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
            Btn("Add a computer", icon = Res.drawable.ic_plus, modifier = Modifier.fillMaxWidth()) { model.startAdding() }
        }
    }
}

@Composable
private fun ComputerRow(computer: Computer, selected: Boolean, onSelect: () -> Unit, onRename: () -> Unit, onUnpair: () -> Unit) {
    val (color, state) = connectionLook(computer)
    Row(
        Modifier.fillMaxWidth().tap(shape = Radius.lg, onClick = onSelect).padding(start = 14.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(Modifier.size(8.dp).clip(Radius.full).background(color))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(computer.name, style = type.label, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (selected) Ico(Res.drawable.ic_check, tint = pal.brand, size = 14.dp)
            }
            Text(state ?: computer.address, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (computer.needsYou > 0) Pill("${computer.needsYou} waiting", color = pal.warning)
        IconBtn(Res.drawable.ic_pencil, onClick = onRename)
        IconBtn(Res.drawable.ic_log_out, onClick = onUnpair)
    }
}

@Composable
private fun RenameRow(computer: Computer, onDone: (String) -> Unit) {
    var name by remember { mutableStateOf(computer.customName ?: computer.name) }
    Row(
        Modifier.fillMaxWidth().padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.weight(1f)) { Field(name, "Name", singleLine = true) { name = it } }
        Btn("Save", tone = Tone.Primary) { onDone(name) }
    }
}

/** One choice in a picker sheet. */
@Composable
private fun PickRow(
    title: String,
    subtitle: String?,
    selected: Boolean,
    titleColor: Color = pal.text,
    onClick: () -> Unit,
    leading: (@Composable () -> Unit)? = null,
) {
    Row(
        Modifier.fillMaxWidth().clip(Radius.md).background(if (selected) pal.surfaceHover else Color.Transparent)
            .tap(onClick = onClick).padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        leading?.invoke()
        Column(Modifier.weight(1f)) {
            Text(title, style = type.label.copy(color = titleColor))
            subtitle?.let { Text(it, style = type.caption.copy(color = pal.textMuted)) }
        }
        if (selected) Ico(Res.drawable.ic_check, size = 17.dp, tint = pal.text)
    }
}

@Composable
private fun PickerBody(title: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).imePadding().padding(horizontal = 12.dp).padding(bottom = 24.dp)) {
        Text(title, style = type.title, modifier = Modifier.padding(start = 12.dp, top = 4.dp, bottom = 12.dp))
        content()
    }
}

@Composable
fun AgentPickerSheet(agents: List<AgentView>, selected: String?, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    Sheet(onDismiss) {
        PickerBody("Agent") {
            agents.forEach { a ->
                PickRow(a.agent.name, a.version, a.agent.id == selected, onClick = { onPick(a.agent.id) }) { AgentAvatar(a.agent, 34.dp) }
            }
        }
    }
}

@Composable
fun ModePickerSheet(modes: List<ModeOption>, selected: String?, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    Sheet(onDismiss) {
        PickerBody("Permissions") {
            modes.forEach { m ->
                PickRow(m.label, m.description, m.id == selected, titleColor = if (m.dangerous) pal.danger else pal.text, onClick = { onPick(m.id) })
            }
        }
    }
}

/**
 * Models to pick from. With a [defaultLabel] the first row is the agent's own default and picks
 * null; without one (a running chat) a model is always chosen.
 */
@Composable
fun ModelPickerSheet(
    models: List<ModelOption>,
    selected: String?,
    onDismiss: () -> Unit,
    defaultLabel: String? = null,
    onPick: (String?) -> Unit,
) {
    Sheet(onDismiss) {
        PickerBody("Model") {
            if (defaultLabel != null) PickRow(defaultLabel, "Let the agent choose", selected == null, onClick = { onPick(null) })
            models.forEach { m ->
                PickRow(m.name, m.description, m.id == selected, onClick = { onPick(m.id) })
            }
        }
    }
}

/** A setting with a few values, such as reasoning effort or context size. Same rules as [ModelPickerSheet]. */
@Composable
fun ChoicePickerSheet(
    title: String,
    options: List<ChoiceOption>,
    selected: String?,
    onDismiss: () -> Unit,
    defaultLabel: String? = null,
    onPick: (String?) -> Unit,
) {
    Sheet(onDismiss) {
        PickerBody(title) {
            if (defaultLabel != null) PickRow(defaultLabel, null, selected == null, onClick = { onPick(null) })
            options.forEach { o -> PickRow(o.label, null, o.id == selected, onClick = { onPick(o.id) }) }
        }
    }
}

@Composable
fun ProjectPickerSheet(model: AppModel, selected: ProjectView?, onDismiss: () -> Unit, onPick: (ProjectView?) -> Unit) {
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<ProjectView>>(emptyList()) }
    LaunchedEffect(query) {
        if (query.isBlank()) { hits = emptyList(); return@LaunchedEffect }
        delay(150)
        hits = model.searchProjects(query)
    }
    Sheet(onDismiss) {
        PickerBody("Project") {
            Box(Modifier.padding(horizontal = 4.dp)) { Field(query, "Search projects", singleLine = true) { query = it } }
            Spacer(Modifier.height(8.dp))
            if (query.isBlank()) ProjectRow(null, selected = selected == null) { onPick(null) }
            val list = if (query.isBlank()) model.projects else hits
            list.forEach { p -> ProjectRow(p, selected = selected?.path == p.path) { onPick(p) } }
            if (query.isNotBlank() && hits.isEmpty()) {
                Text("No projects match", style = type.body.copy(color = pal.textMuted), modifier = Modifier.padding(12.dp))
            }
        }
    }
}

@Composable
private fun ProjectRow(p: ProjectView?, selected: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clip(Radius.md).background(if (selected) pal.surfaceHover else Color.Transparent).tap(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(11.dp),
    ) {
        Ico(if (p == null) Res.drawable.ic_globe else Res.drawable.ic_folder, size = 17.dp, tint = if (selected) pal.text else pal.textMuted)
        Column(Modifier.weight(1f)) {
            Text(p?.name ?: "General", style = type.label)
            Text(p?.displayPath ?: "Not tied to a project · runs from your home folder", style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        p?.branch?.let {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Ico(Res.drawable.ic_git_branch, size = 13.dp, tint = pal.textFaint)
                Text(it, style = type.caption.copy(color = pal.textFaint))
            }
        }
        if (selected) Ico(Res.drawable.ic_check, size = 16.dp, tint = pal.text)
    }
}

@Composable
fun Segmented(options: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Row(Modifier.clip(Radius.md).background(pal.surfaceHover).padding(3.dp)) {
        options.forEachIndexed { i, o ->
            val on = i == selected
            Text(
                o,
                style = type.label.copy(color = if (on) pal.text else pal.textMuted),
                modifier = Modifier.clip(Radius.sm).background(if (on) (if (pal.dark) pal.borderStrong else pal.raised) else Color.Transparent)
                    .tap(shape = Radius.sm) { onSelect(i) }.padding(horizontal = 12.dp, vertical = 6.dp),
            )
        }
    }
}

@Composable
fun UsageSheet(model: AppModel) {
    Sheet({ model.showUsage = false }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Usage & limits", style = type.title, modifier = Modifier.weight(1f))
                IconBtn(Res.drawable.ic_refresh_cw) { model.act { refreshUsage() } }
            }
            Spacer(Modifier.height(8.dp))
            // Opened from a chat: what this chat has used comes first.
            val chat = model.openChat.takeIf { model.stack.isNotEmpty() }
            if (chat != null) {
                Spacer(Modifier.height(6.dp))
                Text("THIS CHAT", style = type.caption.copy(color = pal.textFaint))
                Spacer(Modifier.height(8.dp))
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(11.dp)) {
                            AgentAvatar(chat.summary.agent, 30.dp)
                            Column(Modifier.weight(1f)) {
                                Text(chat.summary.title, style = type.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(
                                    chat.models.firstOrNull { it.id == chat.model }?.name ?: chat.model ?: chat.summary.agent.name,
                                    style = type.caption.copy(color = pal.textFaint),
                                )
                            }
                        }
                        val f = chat.contextFraction
                        val level = if (f >= 0.9f) "critical" else if (f >= 0.75f) "warn" else "ok"
                        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text("Context", style = type.body, modifier = Modifier.weight(1f))
                                Text(
                                    if (chat.contextLabel.isNotEmpty()) "${chat.contextLabel} · ${(f * 100).toInt()}%" else "—",
                                    style = type.caption.copy(color = pal.textMuted),
                                )
                            }
                            Meter(f, levelColor(level, pal), Modifier.fillMaxWidth())
                        }
                        chat.creditsLabel?.let {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f)) {
                                    Text("AI credits used", style = type.body)
                                    Text("GitHub Copilot bills this chat in AIC", style = type.caption.copy(color = pal.textFaint))
                                }
                                Text(it, style = type.label)
                            }
                        }
                        chat.costLabel?.let {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text("Session cost", style = type.body, modifier = Modifier.weight(1f))
                                Text(it, style = type.label)
                            }
                        }
                    }
                }
                Spacer(Modifier.height(10.dp))
                Text("ACCOUNTS", style = type.caption.copy(color = pal.textFaint))
            }
            if (model.usage.isEmpty()) Text("No limit data yet.", style = type.body.copy(color = pal.textMuted))
            // The chat's own agent first.
            model.usage.sortedBy { if (chat != null && it.agent.id == chat.summary.agent.id) 0 else 1 }.forEach { u ->
                Spacer(Modifier.height(14.dp))
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(11.dp)) {
                            AgentAvatar(u.agent, 30.dp)
                            Column(Modifier.weight(1f)) {
                                Text(u.providerName, style = type.label)
                                Text(listOfNotNull(u.plan, "updated ${u.updatedLabel}").joinToString(" · "), style = type.caption.copy(color = pal.textFaint))
                            }
                            if (u.windows.isNotEmpty()) Ring(u.worstFraction, levelColor(u.level, pal), size = 26.dp, stroke = 3.dp)
                        }
                        u.error?.let { Text(it, style = type.caption.copy(color = pal.textMuted)) }
                        u.windows.forEach { w ->
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Text(w.label, style = type.body, modifier = Modifier.weight(1f))
                                    Text(
                                        listOfNotNull(w.amount, "${(w.fraction * 100).toInt()}%").joinToString(" · "),
                                        style = type.caption.copy(color = levelColor(w.level, pal).takeIf { w.level != "ok" } ?: pal.textMuted),
                                    )
                                }
                                Meter(w.fraction, levelColor(w.level, pal), Modifier.fillMaxWidth())
                                val foot = listOfNotNull(w.resets?.let { "Resets $it" }, w.detail).joinToString(" · ")
                                if (foot.isNotEmpty()) Text(foot, style = type.caption.copy(color = pal.textFaint))
                            }
                        }
                    }
                }
            }
            Spacer(Modifier.width(1.dp))
        }
    }
}

/** Theme, accent and tone for this phone. */
@Composable
fun AppearanceSheet(model: AppModel) {
    Sheet({ model.showAppearance = false }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Text("Appearance", style = type.title)
            Spacer(Modifier.height(4.dp))
            Text("How Mothr looks on this phone.", style = type.body.copy(color = pal.textMuted))

            Spacer(Modifier.height(20.dp))
            Text("THEME", style = type.caption.copy(color = pal.textFaint))
            Spacer(Modifier.height(8.dp))
            Segmented(ThemeMode.entries.map { it.label }, ThemeMode.entries.indexOf(model.themeMode)) { model.setLook(mode = ThemeMode.entries[it]) }

            Spacer(Modifier.height(22.dp))
            Text("ACCENT", style = type.caption.copy(color = pal.textFaint))
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                AccentPreset.entries.forEach { a ->
                    val on = a == model.accent
                    val color = c(if (pal.dark) a.dark else a.light)
                    Box(
                        Modifier.size(34.dp).clip(Radius.full).border(2.dp, if (on) color else Color.Transparent, Radius.full)
                            .tap(shape = Radius.full) { model.setLook(accent = a) }.padding(4.dp)
                            .clip(Radius.full).background(color).border(1.dp, pal.borderStrong, Radius.full),
                        contentAlignment = Alignment.Center,
                    ) {
                        if (on) Ico(Res.drawable.ic_check, size = 14.dp, tint = pal.onBrand)
                    }
                }
            }

            Spacer(Modifier.height(22.dp))
            Text("TONE", style = type.caption.copy(color = pal.textFaint))
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                TonePreset.entries.forEach { t ->
                    val on = t == model.tone
                    val look = themedPalette(pal.dark, model.accent, t)
                    Column(
                        Modifier.weight(1f).clip(Radius.md).tap(shape = Radius.md) { model.setLook(tone = t) },
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Row(
                            Modifier.fillMaxWidth().height(52.dp).clip(Radius.md).border(2.dp, if (on) pal.brand else look.border, Radius.md).background(look.bg),
                        ) {
                            Box(Modifier.width(16.dp).fillMaxHeight().background(look.chrome))
                            Column(Modifier.padding(6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Box(Modifier.size(width = 26.dp, height = 7.dp).clip(Radius.sm).background(look.surfaceHover))
                                Box(Modifier.size(width = 16.dp, height = 7.dp).clip(Radius.sm).background(look.brand))
                            }
                        }
                        Text(t.label, style = type.caption.copy(color = if (on) pal.text else pal.textMuted))
                    }
                }
            }
        }
    }
}

private fun c(hex: Long) = Color(0xFF000000 or hex)
