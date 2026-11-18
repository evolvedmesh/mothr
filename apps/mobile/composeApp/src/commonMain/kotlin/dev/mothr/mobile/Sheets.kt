package dev.mothr.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mothr.client.ProjectView
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_check
import dev.mothr.mobile.resources.ic_folder
import dev.mothr.mobile.resources.ic_git_branch
import dev.mothr.mobile.resources.ic_globe
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

@Composable
private fun SectionLabel(text: String) {
    Text(text, style = type.caption.copy(color = pal.textFaint), modifier = Modifier.padding(bottom = 8.dp))
}

@Composable
fun NewChatSheet(model: AppModel) {
    val agents = model.agents.filter { it.available }
    var agentId by remember { mutableStateOf(model.lastAgent?.takeIf { id -> agents.any { it.agent.id == id } } ?: agents.firstOrNull()?.agent?.id) }
    val agent = agents.firstOrNull { it.agent.id == agentId }
    var project by remember { mutableStateOf<ProjectView?>(model.projects.firstOrNull()) }
    var general by remember { mutableStateOf(model.projects.isEmpty()) }
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<ProjectView>>(emptyList()) }
    var modeId by remember { mutableStateOf(model.lastMode ?: "ask") }
    var prompt by remember { mutableStateOf("") }
    var design by remember { mutableStateOf(false) }

    LaunchedEffect(query) {
        if (query.isBlank()) { hits = emptyList(); return@LaunchedEffect }
        delay(150)
        hits = model.searchProjects(query)
    }
    // The mode list depends on the agent; fall back to its closest supported mode.
    val modes = agent?.modes ?: emptyList()
    val mode = modes.firstOrNull { it.id == modeId } ?: modes.firstOrNull()

    Sheet({ model.showNewChat = false }) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).imePadding().padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("New chat", style = type.title, modifier = Modifier.weight(1f))
                Segmented(listOf("Chat", "Design"), if (design) 1 else 0) { design = it == 1 }
            }
            Spacer(Modifier.height(20.dp))

            SectionLabel("Agent")
            if (agents.isEmpty()) {
                Text("No agents are installed on your computer.", style = type.body.copy(color = pal.textMuted))
            }
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                agents.forEach { a ->
                    val on = a.agent.id == agentId
                    Row(
                        Modifier.clip(Radius.lg).background(if (on) pal.surfaceHover else pal.surface)
                            .border(1.dp, if (on) pal.text.copy(alpha = 0.6f) else pal.border, Radius.lg)
                            .tap(shape = Radius.lg) { agentId = a.agent.id }.padding(start = 8.dp, end = 14.dp, top = 8.dp, bottom = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(9.dp),
                    ) {
                        AgentAvatar(a.agent, 28.dp)
                        Text(a.agent.shortName, style = type.label)
                    }
                }
            }
            Spacer(Modifier.height(20.dp))

            SectionLabel("Where")
            Field(query, "Search projects", singleLine = true) { query = it }
            Spacer(Modifier.height(8.dp))
            Column(Modifier.clip(Radius.lg).border(1.dp, pal.border, Radius.lg)) {
                ProjectRow(null, selected = general) { general = true; project = null }
                val list = if (query.isBlank()) model.projects.take(6) else hits
                list.forEach { p -> ProjectRow(p, selected = !general && project?.path == p.path) { general = false; project = p } }
            }
            Spacer(Modifier.height(20.dp))

            SectionLabel("Permissions")
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                modes.forEach { m ->
                    val on = m.id == mode?.id
                    val accent = if (m.dangerous) pal.danger else pal.text
                    Text(
                        m.label,
                        style = type.label.copy(color = if (on) (if (m.dangerous) pal.danger else pal.bg) else pal.textMuted),
                        modifier = Modifier.clip(Radius.full)
                            .background(if (on) (if (m.dangerous) pal.danger.copy(alpha = 0.15f) else accent) else pal.surface)
                            .border(1.dp, if (on) accent.copy(alpha = 0.6f) else pal.border, Radius.full)
                            .tap(shape = Radius.full) { modeId = m.id }
                            .padding(horizontal = 14.dp, vertical = 9.dp),
                    )
                }
            }
            mode?.let { Text(it.description, style = type.caption.copy(color = pal.textMuted), modifier = Modifier.padding(top = 8.dp)) }
            Spacer(Modifier.height(20.dp))

            SectionLabel(if (design) "What should it design?" else "First message")
            Box(Modifier.fillMaxWidth().heightIn(min = 96.dp)) {
                Field(prompt, if (design) "A design system for a calm banking app…" else "What are we working on?") { prompt = it }
            }
            Spacer(Modifier.height(18.dp))
            Btn(
                "Start",
                tone = Tone.Primary,
                enabled = agent != null && mode != null && (general || project != null),
                modifier = Modifier.fillMaxWidth(),
            ) {
                model.newChat(agent!!.agent.id, if (general) "" else project!!.path, mode!!.id, prompt, design)
            }
        }
    }
}

@Composable
private fun ProjectRow(p: ProjectView?, selected: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().background(if (selected) pal.surfaceHover else pal.raised).tap(shape = Radius.sm, onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 11.dp),
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
                modifier = Modifier.clip(Radius.sm).background(if (on) pal.raised else androidx.compose.ui.graphics.Color.Transparent)
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
                Text("Limits", style = type.title, modifier = Modifier.weight(1f))
                IconBtn(Res.drawable.ic_refresh_cw) { model.act { refreshUsage() } }
            }
            Spacer(Modifier.height(8.dp))
            if (model.usage.isEmpty()) Text("No limit data yet.", style = type.body.copy(color = pal.textMuted))
            model.usage.forEach { u ->
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
