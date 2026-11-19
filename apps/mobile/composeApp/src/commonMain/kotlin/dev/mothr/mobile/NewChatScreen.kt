package dev.mothr.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.mothr.client.ProjectView
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_arrow_up
import dev.mothr.mobile.resources.ic_brain
import dev.mothr.mobile.resources.ic_folder
import dev.mothr.mobile.resources.ic_globe
import dev.mothr.mobile.resources.ic_layers
import dev.mothr.mobile.resources.ic_shield_alert
import dev.mothr.mobile.resources.ic_sparkles
import dev.mothr.mobile.resources.ic_x
import kotlinx.coroutines.delay
import org.jetbrains.compose.resources.DrawableResource

private enum class Picker { Agent, Project, Mode, Model, Effort, Context }

/** A project choice; a null [project] means General, not tied to any project. */
private data class ProjectPick(val project: ProjectView?)

/** A choice that may be Default: a null [id] is a real answer, unlike no pick at all. */
private data class Choice(val id: String?)

private data class Starter(val label: String, val text: String)

private val chatStarters = listOf(
    Starter("Fix a bug", "Find and fix the bug where "),
    Starter("Explain this project", "Explain how this project is structured."),
    Starter("Write tests", "Write tests for "),
    Starter("Review my changes", "Review my uncommitted changes."),
)

private val designStarters = listOf(
    Starter("Landing page", "Design a landing page for "),
    Starter("App screen", "Design a mobile app screen for "),
    Starter("Design system", "Create a design system for "),
)

/**
 * Starting a chat: write first, like any messaging app. The large field has the keyboard from
 * the start, and the agent, project and permissions sit in chips with sensible defaults.
 */
@Composable
fun NewChatScreen(model: AppModel) {
    val agents = model.agents.filter { it.available }
    var agentId by remember { mutableStateOf<String?>(null) }
    val agent = agents.firstOrNull { it.agent.id == agentId }
        ?: agents.firstOrNull { it.agent.id == model.lastAgent }
        ?: agents.firstOrNull()

    // The mode list depends on the agent; fall back to its closest supported mode.
    var modeId by remember { mutableStateOf<String?>(null) }
    val modes = agent?.modes.orEmpty()
    val wantedMode = modeId ?: model.lastMode ?: "ask"
    val mode = modes.firstOrNull { it.id == wantedMode } ?: modes.firstOrNull()

    // Model, reasoning and context: what the person picked this time, else what this agent last
    // started with. Whatever the agent (or the picked model) no longer offers falls back to Default.
    var modelChoice by remember { mutableStateOf<Choice?>(null) }
    var effortChoice by remember { mutableStateOf<Choice?>(null) }
    var contextChoice by remember { mutableStateOf<Choice?>(null) }
    val agentKey = agent?.agent?.id
    val modelOption = agent?.models?.firstOrNull { it.id == (modelChoice ?: Choice(agentKey?.let(model::lastModel))).id }
    val effort = modelOption?.efforts?.firstOrNull { it.id == (effortChoice ?: Choice(agentKey?.let(model::lastEffort))).id }
    val context = modelOption?.contexts?.firstOrNull { it.id == (contextChoice ?: Choice(agentKey?.let(model::lastContext))).id }

    var picked by remember { mutableStateOf<ProjectPick?>(null) }
    val last = model.lastProject
    val project = when {
        picked != null -> picked?.project
        last == "" -> null
        else -> model.projects.firstOrNull { it.path == last } ?: model.projects.firstOrNull()
    }

    var design by remember { mutableStateOf(false) }
    var field by remember { mutableStateOf(TextFieldValue(model.newChatDraft, TextRange(model.newChatDraft.length))) }
    fun setText(value: TextFieldValue) {
        field = value
        model.newChatDraft = value.text
    }

    var picker by remember { mutableStateOf<Picker?>(null) }
    val focus = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(picker) {
        if (picker == null) {
            delay(120)
            focus.requestFocus()
            keyboard?.show()
        }
    }
    fun open(p: Picker) {
        focusManager.clearFocus()
        picker = p
    }

    val computer = model.current?.name.orEmpty()
    val loaded = model.connection?.loaded == true
    val problem = when {
        !loaded -> "Connecting to $computer…"
        agents.isEmpty() -> "No agents are installed on $computer."
        else -> null
    }
    val canSend = agent != null && mode != null && field.text.isNotBlank() && !model.creatingChat
    val send = {
        if (canSend) {
            model.newChat(agent.agent.id, project?.path.orEmpty(), mode.id, modelOption?.id, effort?.id, context?.id, field.text.trim(), design)
        }
    }

    val insets = WindowInsets.statusBars.union(WindowInsets.ime.union(WindowInsets.navigationBars))
    Column(Modifier.fillMaxSize().background(pal.bg).windowInsetsPadding(insets)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 6.dp, end = 14.dp, top = 6.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            IconBtn(Res.drawable.ic_x, tint = pal.text) { model.showNewChat = false }
            Column(Modifier.weight(1f)) {
                Text("New chat", style = type.heading, maxLines = 1)
                Text("on $computer", style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Segmented(listOf("Chat", "Design"), if (design) 1 else 0) { design = it == 1 }
        }

        Box(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 20.dp, vertical = 14.dp)) {
            if (field.text.isEmpty()) {
                val who = agent?.agent?.shortName ?: "the agent"
                Text(
                    if (design) "What should $who design?" else "What should $who work on?",
                    style = type.prompt.copy(color = pal.textFaint),
                )
            }
            BasicTextField(
                value = field,
                onValueChange = ::setText,
                textStyle = type.prompt,
                cursorBrush = SolidColor(pal.brand),
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                modifier = Modifier.fillMaxSize().focusRequester(focus),
            )
        }

        if (field.text.isEmpty()) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                (if (design) designStarters else chatStarters).forEach { s ->
                    Text(
                        s.label,
                        style = type.label.copy(color = pal.textMuted),
                        maxLines = 1,
                        modifier = Modifier.clip(Radius.full).border(1.dp, pal.border, Radius.full)
                            .tap(shape = Radius.full) { setText(TextFieldValue(s.text, TextRange(s.text.length))) }
                            .padding(horizontal = 14.dp, vertical = 9.dp),
                    )
                }
            }
        }

        problem?.let {
            Text(it, style = type.caption.copy(color = pal.textMuted), modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 6.dp))
        }

        Row(
            Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 2.dp, bottom = 10.dp).clip(Radius.xl).background(pal.surface)
                .border(1.dp, pal.borderStrong, Radius.xl).padding(start = 6.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                agent?.let { a ->
                    Chip(a.agent.shortName, onClick = { open(Picker.Agent) }) { AgentAvatar(a.agent, 20.dp) }
                }
                Chip(project?.name ?: "General", onClick = { open(Picker.Project) }) {
                    ChipIcon(if (project == null) Res.drawable.ic_globe else Res.drawable.ic_folder, pal.textMuted)
                }
                if (agent != null) {
                    Chip(modelOption?.name ?: "Model", onClick = { open(Picker.Model) }) {
                        ChipIcon(Res.drawable.ic_sparkles, pal.textMuted)
                    }
                    if (modelOption?.efforts?.isNotEmpty() == true) {
                        Chip(effort?.label ?: "Reasoning", onClick = { open(Picker.Effort) }) {
                            ChipIcon(Res.drawable.ic_brain, pal.textMuted)
                        }
                    }
                    if (modelOption?.contexts?.isNotEmpty() == true) {
                        Chip(context?.label ?: "Context", onClick = { open(Picker.Context) }) {
                            ChipIcon(Res.drawable.ic_layers, pal.textMuted)
                        }
                    }
                }
                mode?.let { m ->
                    val color = if (m.dangerous) pal.danger else pal.textMuted
                    Chip(m.label, color = if (m.dangerous) pal.danger else pal.text, onClick = { open(Picker.Mode) }) {
                        ChipIcon(Res.drawable.ic_shield_alert, color)
                    }
                }
            }
            Box(
                Modifier.size(44.dp).clip(Radius.full).background(if (canSend) pal.text else pal.surfaceHover)
                    .tap(enabled = canSend, shape = Radius.full, onClick = { send() }),
                contentAlignment = Alignment.Center,
            ) {
                if (model.creatingChat) {
                    CircularProgressIndicator(Modifier.size(18.dp), color = pal.textMuted, strokeWidth = 2.dp)
                } else {
                    Ico(Res.drawable.ic_arrow_up, tint = if (canSend) pal.bg else pal.textFaint, size = 20.dp)
                }
            }
        }
    }

    when (picker) {
        Picker.Agent -> AgentPickerSheet(agents, agent?.agent?.id, onDismiss = { picker = null }) {
            if (it != agent?.agent?.id) {
                // Each agent has its own models; start from what it last used.
                modelChoice = null
                effortChoice = null
                contextChoice = null
            }
            agentId = it
            picker = null
        }
        Picker.Project -> ProjectPickerSheet(model, project, onDismiss = { picker = null }) {
            picked = ProjectPick(it)
            picker = null
        }
        Picker.Mode -> ModePickerSheet(modes, mode?.id, onDismiss = { picker = null }) {
            modeId = it
            picker = null
        }
        Picker.Model -> ModelPickerSheet(agent?.models.orEmpty(), modelOption?.id, onDismiss = { picker = null }, defaultLabel = "Default") {
            modelChoice = Choice(it)
            picker = null
        }
        Picker.Effort -> ChoicePickerSheet("Reasoning", modelOption?.efforts.orEmpty(), effort?.id, onDismiss = { picker = null }, defaultLabel = "Default") {
            effortChoice = Choice(it)
            picker = null
        }
        Picker.Context -> ChoicePickerSheet("Context", modelOption?.contexts.orEmpty(), context?.id, onDismiss = { picker = null }, defaultLabel = "Default") {
            contextChoice = Choice(it)
            picker = null
        }
        null -> {}
    }
}

@Composable
private fun ChipIcon(icon: DrawableResource, tint: Color) {
    Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) { Ico(icon, tint = tint, size = 16.dp) }
}

/** A filled pill, so it reads as a button and as cut off when the row scrolls past the edge. */
@Composable
private fun Chip(label: String, color: Color = pal.text, onClick: () -> Unit, leading: @Composable () -> Unit) {
    Row(
        Modifier.clip(Radius.full).background(pal.surfaceHover).tap(shape = Radius.full, onClick = onClick)
            .padding(start = 7.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        leading()
        Text(label, style = type.label.copy(color = color), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 120.dp))
    }
}
