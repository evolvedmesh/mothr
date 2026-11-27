package dev.mothr.mobile

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import com.mikepenz.markdown.model.rememberMarkdownState
import kotlinx.coroutines.delay
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mikepenz.markdown.m3.Markdown
import com.mikepenz.markdown.m3.markdownColor
import com.mikepenz.markdown.m3.markdownTypography
import dev.mothr.client.ChatDetail
import dev.mothr.client.ChatStatus
import dev.mothr.client.ConnectionState
import dev.mothr.client.Delivery
import dev.mothr.client.EntryView
import dev.mothr.client.ToolState
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_arrow_down
import dev.mothr.mobile.resources.ic_arrow_left
import dev.mothr.mobile.resources.ic_arrow_up
import dev.mothr.mobile.resources.ic_book_open
import dev.mothr.mobile.resources.ic_gauge
import dev.mothr.mobile.resources.ic_bot
import dev.mothr.mobile.resources.ic_brain
import dev.mothr.mobile.resources.ic_check
import dev.mothr.mobile.resources.ic_chevron_down
import dev.mothr.mobile.resources.ic_chevron_right
import dev.mothr.mobile.resources.ic_circle_alert
import dev.mothr.mobile.resources.ic_corner_down_right
import dev.mothr.mobile.resources.ic_globe
import dev.mothr.mobile.resources.ic_info
import dev.mothr.mobile.resources.ic_list_checks
import dev.mothr.mobile.resources.ic_message_circle_question_mark
import dev.mothr.mobile.resources.ic_pencil
import dev.mothr.mobile.resources.ic_play
import dev.mothr.mobile.resources.ic_search
import dev.mothr.mobile.resources.ic_shield_alert
import dev.mothr.mobile.resources.ic_sparkles
import dev.mothr.mobile.resources.ic_square
import dev.mothr.mobile.resources.ic_terminal
import dev.mothr.mobile.resources.ic_wifi_off
import dev.mothr.mobile.resources.ic_x
import dev.mothr.mobile.resources.ic_zap
import kotlinx.coroutines.launch
import org.jetbrains.compose.resources.DrawableResource

@Composable
fun ChatScreen(model: AppModel) {
    val chat = model.openChat
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
        if (chat == null) {
            TopBar(model, null)
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Opening…", style = type.body.copy(color = pal.textMuted)) }
            return@Column
        }
        TopBar(model, chat)
        if (chat.subagents.isNotEmpty()) SubagentStrip(model, chat)
        Box(Modifier.weight(1f)) { Transcript(model, chat) }
        val asking = chat.entries.filterIsInstance<EntryView.Question>().firstOrNull { it.answers == null }
        if (asking != null) QuestionDock(model, chat, asking) else Composer(model, chat)
    }
}

@Composable
private fun TopBar(model: AppModel, chat: ChatDetail?) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 4.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconBtn(Res.drawable.ic_arrow_left, tint = pal.text) { model.back() }
        if (chat == null) return@Row
        val s = chat.summary
        AgentAvatar(s.agent, 30.dp)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(s.title, style = type.heading, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                val offline = model.connection?.state.let { it != null && it != ConnectionState.CONNECTED }
                if (offline) {
                    // What's on screen may be stale until the link is back.
                    Ico(Res.drawable.ic_wifi_off, tint = pal.warning, size = 12.dp)
                    Text("Reconnecting…", style = type.caption.copy(color = pal.warning), maxLines = 1)
                } else {
                    StatusDot(s.status, 6.dp)
                    Text(
                        listOf(s.statusLabel, s.projectName, chat.models.firstOrNull { it.id == chat.model }?.name ?: chat.model ?: s.agent.shortName).joinToString(" · "),
                        style = type.caption.copy(color = pal.textFaint),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        // Context and credits in one control; tapping opens this chat's usage and the account limits.
        if (chat.contextLabel.isNotEmpty() || chat.creditsLabel != null) {
            Row(
                Modifier.clip(Radius.full).background(pal.surfaceHover).tap(shape = Radius.full) { model.showUsage = true }
                    .padding(horizontal = 9.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (chat.contextLabel.isNotEmpty()) {
                    val f = chat.contextFraction
                    Ring(f, if (f >= 0.9f) pal.danger else if (f >= 0.75f) pal.warning else pal.textMuted, size = 14.dp, stroke = 2.dp)
                    Text("${(f * 100).toInt()}%", style = type.caption)
                }
                chat.creditsLabel?.let {
                    if (chat.contextLabel.isNotEmpty()) Box(Modifier.size(width = 1.dp, height = 12.dp).background(pal.borderStrong))
                    Text(it, style = type.caption)
                }
            }
        } else {
            IconBtn(Res.drawable.ic_gauge, tint = pal.textMuted) { model.showUsage = true }
        }
        IconBtn(Res.drawable.ic_terminal, tint = pal.textMuted) { model.showTerminal(s.id) }
    }
    Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
}

@Composable
private fun SubagentStrip(model: AppModel, chat: ChatDetail) {
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        chat.subagents.forEach { sub ->
            Row(
                Modifier.clip(Radius.full).background(pal.surfaceHover).tap(shape = Radius.full) { model.open(sub.id) }
                    .padding(horizontal = 11.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                StatusDot(sub.status, 7.dp)
                Text(sub.title, style = type.caption.copy(color = pal.text), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 180.dp))
            }
        }
    }
    Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
}

@Composable
private fun Transcript(model: AppModel, chat: ChatDetail) {
    // Bottom-anchored (reverse) layout: index 0 is the newest entry. While `following`, the view
    // stays glued to the bottom as entries arrive and the last one grows; the person scrolling up
    // lets go, and the jump button (or sending a message) takes it up again.
    val id = chat.summary.id
    val list = key(id) { rememberLazyListState() }
    var following by remember(id) { mutableStateOf(true) }
    val scope = rememberCoroutineScope()
    val nearBottom = with(LocalDensity.current) { 24.dp.toPx() }
    val atBottom = { list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset <= nearBottom }
    // Only gestures (drags and the flings after them) reach this connection, never our own
    // programmatic scrolls, so only the person can stop or resume following by scrolling.
    val userScroll = remember(list) {
        object : NestedScrollConnection {
            override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
                following = atBottom()
                return Offset.Zero
            }

            override suspend fun onPostFling(consumed: Velocity, available: Velocity): Velocity {
                following = atBottom()
                return Velocity.Zero
            }
        }
    }
    val entries = chat.entries
    if (entries.isEmpty()) {
        Column(Modifier.fillMaxSize().padding(32.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
            Text(if (chat.summary.status == ChatStatus.STARTING) "Starting ${chat.summary.agent.name}…" else "Send a message to start", style = type.body.copy(color = pal.textMuted))
        }
        return
    }
    // A new entry or growth of the last one (streaming text, tool output): stay at the bottom.
    // The list keeps its place relative to the old newest item, so a new entry would otherwise
    // land just below the screen.
    val tail = entries.last()
    LaunchedEffect(entries.size, tail) { if (following) list.scrollToItem(0) }
    // Sending a message always takes you to it.
    val lastUser = entries.lastOrNull { it is EntryView.User }?.let { entryId(it) }
    LaunchedEffect(lastUser) {
        following = true
        if (list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 0) list.animateScrollToItem(0)
    }
    Box(Modifier.fillMaxSize()) {
        SelectionContainer {
            LazyColumn(
                state = list,
                reverseLayout = true,
                modifier = Modifier.fillMaxSize().nestedScroll(userScroll),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Bottom),
            ) {
                items(entries.asReversed(), key = { entryId(it) }) { e ->
                    // What arrives while the chat is open fades in; nothing slides, since the newest
                    // entry grows as it streams and the ones above would trail it.
                    Box(Modifier.animateItem(fadeInSpec = tween(260), placementSpec = null, fadeOutSpec = null)) { EntryItem(model, chat, e) }
                }
            }
        }
        AnimatedVisibility(
            visible = !following,
            enter = fadeIn() + scaleIn(initialScale = 0.8f),
            exit = fadeOut() + scaleOut(targetScale = 0.8f),
            modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 12.dp),
        ) {
            Box(
                Modifier.size(40.dp).clip(Radius.full).background(pal.raised).border(1.dp, pal.borderStrong, Radius.full)
                    .tap(shape = Radius.full) {
                        following = true
                        scope.launch { list.animateScrollToItem(0) }
                    },
                contentAlignment = Alignment.Center,
            ) { Ico(Res.drawable.ic_arrow_down, tint = pal.text, size = 20.dp) }
        }
    }
}

private fun entryId(e: EntryView): String = when (e) {
    is EntryView.User -> e.id
    is EntryView.Assistant -> e.id
    is EntryView.Thinking -> e.id
    is EntryView.Tool -> e.id
    is EntryView.Plan -> e.id
    is EntryView.Permission -> e.id
    is EntryView.Question -> e.id
    is EntryView.Notice -> e.id
    is EntryView.Image -> e.id
    is EntryView.TurnEnd -> e.id
}

@Composable
private fun EntryItem(model: AppModel, chat: ChatDetail, e: EntryView) {
    when (e) {
        is EntryView.User -> Column(
            Modifier.fillMaxWidth().padding(start = 44.dp),
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            e.images.forEach { ChatImage(model, it, "Attached image", maxWidth = 220.dp) }
            if (e.text.isNotEmpty() || e.images.isEmpty()) Box(
                Modifier.clip(Radius.xl).background(pal.surfaceHover).padding(horizontal = 15.dp, vertical = 11.dp),
            ) { Text(e.text, style = type.reading) }
        }
        is EntryView.Assistant -> MarkdownText(revealed(e.id, e.text, e.streaming), Modifier.animateContentSize(tween(200)))
        is EntryView.Thinking -> Thinking(e)
        is EntryView.Tool -> ToolCard(e)
        is EntryView.Plan -> PlanCard(e)
        is EntryView.Permission -> PermissionCard(model, chat, e)
        is EntryView.Question -> QuestionCard(e)
        is EntryView.Notice -> Notice(e.level, e.text)
        is EntryView.Image -> ChatImage(model, e.name, e.label)
        is EntryView.TurnEnd -> if (e.summary.isNotEmpty()) {
            Text(e.summary, style = type.caption.copy(color = pal.textFaint), modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center)
        }
    }
}

@Composable
private fun Notice(level: String, text: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
        val color = when (level) { "error" -> pal.danger; "warning" -> pal.warning; else -> pal.textMuted }
        Ico(if (level == "info") Res.drawable.ic_info else Res.drawable.ic_circle_alert, tint = color, size = 16.dp)
        Text(text, style = type.body.copy(color = color))
    }
}

private sealed interface Pic {
    data object Loading : Pic
    data object Missing : Pic
    class Ready(val bitmap: ImageBitmap) : Pic
}

/**
 * A picture in the transcript, fetched from the desktop by its stored [name]. Very tall or wide
 * ones are cropped to a sane shape here; tapping opens the whole picture full screen.
 */
@Composable
private fun ChatImage(model: AppModel, name: String?, label: String, maxWidth: Dp = 320.dp) {
    val pic by produceState<Pic>(Pic.Loading, name, model.current) {
        value = name?.let { model.image(it) }?.let { Pic.Ready(it) } ?: Pic.Missing
    }
    var viewing by remember { mutableStateOf(false) }
    when (val p = pic) {
        Pic.Loading -> Box(Modifier.size(width = maxWidth * 0.6f, height = maxWidth * 0.45f).clip(Radius.lg).background(pal.surfaceHover))
        Pic.Missing -> Notice("info", label)
        is Pic.Ready -> {
            val aspect = (p.bitmap.width.toFloat() / p.bitmap.height.coerceAtLeast(1)).coerceIn(0.6f, 2.4f)
            Image(
                p.bitmap,
                contentDescription = label,
                contentScale = ContentScale.Crop,
                alignment = Alignment.TopCenter,
                modifier = Modifier.widthIn(max = maxWidth).fillMaxWidth().aspectRatio(aspect)
                    .clip(Radius.lg).border(1.dp, pal.border, Radius.lg).tap(shape = Radius.lg) { viewing = true },
            )
            if (viewing) ImageViewer(p.bitmap, label) { viewing = false }
        }
    }
}

/** The whole picture on black: pinch or double-tap to zoom, drag to pan, tap to close. */
@Composable
private fun ImageViewer(bitmap: ImageBitmap, label: String, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        var scale by remember { mutableStateOf(1f) }
        var offset by remember { mutableStateOf(Offset.Zero) }
        var size by remember { mutableStateOf(IntSize.Zero) }
        // Keep the zoomed picture over the screen: it can pan at most its overflow.
        fun clamp(o: Offset, s: Float): Offset {
            val maxX = size.width * (s - 1f) / 2f
            val maxY = size.height * (s - 1f) / 2f
            return Offset(o.x.coerceIn(-maxX, maxX), o.y.coerceIn(-maxY, maxY))
        }
        Box(
            Modifier.fillMaxSize().background(Color.Black).onSizeChanged { size = it }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onDoubleTap = { if (scale > 1f) { scale = 1f; offset = Offset.Zero } else scale = 2.5f },
                        onTap = { if (scale <= 1f) onDismiss() },
                    )
                }
                .pointerInput(Unit) {
                    detectTransformGestures { _, pan, zoom, _ ->
                        scale = (scale * zoom).coerceIn(1f, 6f)
                        offset = clamp(offset + pan, scale)
                    }
                },
        ) {
            Image(
                bitmap,
                contentDescription = label,
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize().graphicsLayer {
                    scaleX = scale
                    scaleY = scale
                    translationX = offset.x
                    translationY = offset.y
                },
            )
            Box(Modifier.align(Alignment.TopEnd).windowInsetsPadding(WindowInsets.statusBars).padding(8.dp)) {
                IconBtn(Res.drawable.ic_x, tint = Color.White, bg = Color.Black.copy(alpha = 0.45f), onClick = onDismiss)
            }
        }
    }
}

/**
 * Streamed text as it shows this frame. An agent's text lands a dozen words at a time; revealing
 * each chunk word by word over about a quarter of a second reads as writing, not popping in.
 * Text that is already there when the entry first shows (an older message) shows at once.
 */
@Composable
private fun revealed(id: String, text: String, streaming: Boolean): String {
    var shown by rememberSaveable(id) { mutableIntStateOf(if (streaming) 0 else text.length) }
    LaunchedEffect(id, text) {
        while (shown < text.length) {
            delay(REVEAL_TICK_MS)
            // Close what is left in REVEAL_STEPS ticks, however large, and end on a word.
            var next = minOf(text.length, shown + maxOf(4, (text.length - shown) / REVEAL_STEPS))
            while (next < text.length && !text[next].isWhitespace()) next++
            shown = next
        }
    }
    return if (shown >= text.length) text else text.substring(0, shown)
}

private const val REVEAL_TICK_MS = 32L
private const val REVEAL_STEPS = 8

@Composable
fun MarkdownText(text: String, modifier: Modifier = Modifier) {
    val p = pal
    val t = type
    // Keep the last render while new text parses, or every streamed chunk blanks the message
    // for a frame.
    val state = rememberMarkdownState(text, retainState = true)
    Markdown(
        state,
        modifier = modifier,
        colors = markdownColor(text = p.text, codeBackground = p.codeBg, inlineCodeBackground = p.surfaceHover, dividerColor = p.border),
        typography = markdownTypography(
            h1 = t.title, h2 = t.title, h3 = t.heading, h4 = t.heading, h5 = t.label, h6 = t.label,
            text = t.reading, paragraph = t.reading, code = t.mono, inlineCode = t.mono.copy(fontSize = t.reading.fontSize * 0.9f),
            quote = t.reading.copy(color = p.textMuted), bullet = t.reading, list = t.reading, ordered = t.reading,
            textLink = androidx.compose.ui.text.TextLinkStyles(style = androidx.compose.ui.text.SpanStyle(color = p.brand)),
        ),
    )
}

@Composable
private fun Thinking(e: EntryView.Thinking) {
    var open by remember { mutableStateOf(false) }
    Column(Modifier.animateContentSize()) {
        Row(Modifier.tap { open = !open }.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            Ico(Res.drawable.ic_brain, size = 15.dp, tint = if (e.streaming) pal.brand else pal.textFaint)
            Text(if (e.streaming) "Thinking…" else "Thought", style = type.caption.copy(color = pal.textMuted))
            Ico(if (open) Res.drawable.ic_chevron_down else Res.drawable.ic_chevron_right, size = 14.dp, tint = pal.textFaint)
        }
        if (open) Text(e.text, style = type.body.copy(color = pal.textMuted), modifier = Modifier.padding(start = 22.dp, top = 4.dp))
    }
}

private fun toolIcon(kind: String): DrawableResource = when (kind) {
    "read" -> Res.drawable.ic_book_open
    "edit", "delete", "move" -> Res.drawable.ic_pencil
    "execute" -> Res.drawable.ic_terminal
    "search" -> Res.drawable.ic_search
    "fetch" -> Res.drawable.ic_globe
    "task" -> Res.drawable.ic_bot
    "think" -> Res.drawable.ic_brain
    else -> Res.drawable.ic_zap
}

@Composable
private fun ToolCard(e: EntryView.Tool) {
    var open by remember { mutableStateOf(false) }
    val expandable = e.output != null || e.diff != null
    Card(Modifier.fillMaxWidth().animateContentSize()) {
        Column {
            Row(
                Modifier.fillMaxWidth().tap(enabled = expandable) { open = !open }.padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Ico(toolIcon(e.kind), size = 16.dp)
                Column(Modifier.weight(1f)) {
                    Text(e.title, style = type.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    e.input?.let { Text(it, style = type.mono.copy(color = pal.textMuted, fontSize = type.mono.fontSize * 0.92f), maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
                if (e.added > 0u || e.removed > 0u) {
                    Text("+${e.added}", style = type.mono.copy(color = pal.success))
                    Text("−${e.removed}", style = type.mono.copy(color = pal.danger))
                }
                when (e.status) {
                    ToolState.RUNNING, ToolState.PENDING -> StatusDot(ChatStatus.WORKING, 7.dp)
                    ToolState.DONE -> Ico(Res.drawable.ic_check, tint = pal.success, size = 15.dp)
                    ToolState.FAILED -> Ico(Res.drawable.ic_x, tint = pal.danger, size = 15.dp)
                }
            }
            if (open) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
                e.diff?.let { DiffBlock(it) }
                e.output?.let { CodeBlock(it) }
            }
        }
    }
}

@Composable
private fun CodeBlock(text: String) {
    Box(Modifier.fillMaxWidth().background(pal.codeBg).heightIn(max = 360.dp).horizontalScroll(rememberScrollState()).padding(12.dp)) {
        Text(text, style = type.mono.copy(fontSize = type.mono.fontSize * 0.92f))
    }
}

@Composable
private fun DiffBlock(diff: String) {
    Column(Modifier.fillMaxWidth().background(pal.codeBg).heightIn(max = 360.dp).horizontalScroll(rememberScrollState()).padding(vertical = 8.dp)) {
        diff.lineSequence().filter { it.isNotEmpty() }.forEach { line ->
            val (fg, bg) = when (line.first()) {
                '+' -> pal.success to pal.success.copy(alpha = 0.08f)
                '-' -> pal.danger to pal.danger.copy(alpha = 0.08f)
                else -> pal.textMuted to Color.Transparent
            }
            Text(line, style = type.mono.copy(color = fg, fontSize = type.mono.fontSize * 0.92f), modifier = Modifier.background(bg).padding(horizontal = 12.dp, vertical = 1.dp))
        }
    }
}

@Composable
private fun PlanCard(e: EntryView.Plan) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Ico(Res.drawable.ic_list_checks, size = 16.dp)
                Text("Plan", style = type.label)
                Text("${e.items.count { it.status == "completed" }}/${e.items.size}", style = type.caption.copy(color = pal.textFaint))
            }
            e.items.forEach { item ->
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Top) {
                    val done = item.status == "completed"
                    val active = item.status == "in_progress"
                    Box(
                        Modifier.padding(top = 3.dp).size(15.dp).clip(Radius.full)
                            .background(if (done) pal.success.copy(alpha = 0.18f) else Color.Transparent)
                            .border(1.5.dp, if (done) Color.Transparent else if (active) pal.brand else pal.borderStrong, Radius.full),
                        contentAlignment = Alignment.Center,
                    ) { if (done) Ico(Res.drawable.ic_check, tint = pal.success, size = 10.dp) }
                    Text(
                        item.content,
                        style = type.body.copy(color = if (done) pal.textFaint else pal.text, fontWeight = if (active) FontWeight.Medium else FontWeight.Normal),
                    )
                }
            }
        }
    }
}

@Composable
private fun PermissionCard(model: AppModel, chat: ChatDetail, e: EntryView.Permission) {
    val resolved = e.resolved
    if (resolved != null) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Ico(Res.drawable.ic_shield_alert, size = 15.dp, tint = pal.textFaint)
            Text("${e.title} · $resolved", style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        return
    }
    Card(Modifier.fillMaxWidth(), border = pal.warning.copy(alpha = 0.55f), bg = pal.warning.copy(alpha = 0.06f)) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Ico(Res.drawable.ic_shield_alert, tint = pal.warning, size = 17.dp)
                Text(e.title, style = type.label, modifier = Modifier.weight(1f))
            }
            e.detail?.let {
                Box(Modifier.fillMaxWidth().clip(Radius.md).background(pal.codeBg).padding(10.dp)) {
                    Text(it, style = type.mono, maxLines = 12, overflow = TextOverflow.Ellipsis)
                }
            }
            FlowRowSpaced {
                e.options.forEach { o ->
                    val tone = when {
                        o.allow && !o.always -> Tone.Warning
                        o.allow -> Tone.Quiet
                        else -> Tone.Ghost
                    }
                    Btn(o.label, tone = tone) { model.act { respondPermission(chat.summary.id, e.id, o.id) } }
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FlowRowSpaced(content: @Composable () -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { content() }
}

/** The transcript keeps a quiet summary; a pending question is answered in [QuestionDock]. */
@Composable
private fun QuestionCard(e: EntryView.Question) {
    val answers = e.answers
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (answers == null) {
                Text("Waiting for your answer below", style = type.body.copy(color = pal.textMuted))
            } else {
                e.questions.forEachIndexed { i, q ->
                    Text(q.question, style = type.body.copy(color = pal.textMuted))
                    Text(answers.getOrNull(i)?.joinToString(", ")?.ifEmpty { "Skipped" } ?: "Skipped", style = type.label)
                }
            }
        }
    }
}

/** A pending question, one at a time, in place of the composer. */
@Composable
private fun QuestionDock(model: AppModel, chat: ChatDetail, e: EntryView.Question) {
    val n = e.questions.size
    if (n == 0) return
    var step by remember(e.id) { mutableStateOf(0) }
    val picks = remember(e.id) { e.questions.map { mutableStateListOf<String>() } }
    val other = remember(e.id) { e.questions.map { mutableStateOf("") } }
    val i = step.coerceIn(0, n - 1)
    val q = e.questions[i]
    val last = i == n - 1
    val ready = picks[i].isNotEmpty() || other[i].value.isNotBlank()
    val insets = WindowInsets.ime.union(WindowInsets.navigationBars)
    Column(Modifier.fillMaxWidth().background(pal.bg).windowInsetsPadding(insets).padding(start = 12.dp, end = 12.dp, top = 6.dp, bottom = 10.dp)) {
        Card(Modifier.fillMaxWidth(), border = pal.brand.copy(alpha = 0.55f), bg = pal.surface) {
            Column(Modifier.padding(14.dp).heightIn(max = 460.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Ico(Res.drawable.ic_message_circle_question_mark, tint = pal.brand, size = 16.dp)
                    Text(q.header ?: "Question", style = type.caption.copy(color = pal.brand), modifier = Modifier.weight(1f))
                    if (q.multiSelect) Text("choose any", style = type.caption.copy(color = pal.textFaint))
                    if (n > 1) Text("${i + 1} of $n", style = type.caption.copy(color = pal.textFaint))
                }
                Text(q.question, style = type.reading.copy(fontWeight = FontWeight.Medium))
                q.options.forEach { o ->
                    val on = o.label in picks[i]
                    Row(
                        Modifier.fillMaxWidth().clip(Radius.md).background(if (on) pal.brandSoft else pal.surface)
                            .border(1.dp, if (on) pal.brand else pal.borderStrong, Radius.md)
                            .tap(shape = Radius.md) {
                                if (q.multiSelect) { if (on) picks[i].remove(o.label) else picks[i].add(o.label) }
                                else { picks[i].clear(); picks[i].add(o.label) }
                            }
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Box(
                            Modifier.padding(top = 3.dp).size(14.dp)
                                .clip(if (q.multiSelect) Radius.sm else Radius.full)
                                .background(if (on) pal.brand else Color.Transparent)
                                .border(1.5.dp, if (on) pal.brand else pal.textFaint, if (q.multiSelect) Radius.sm else Radius.full),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(o.label, style = type.label)
                            o.description?.let { Text(it, style = type.caption.copy(color = pal.textMuted)) }
                        }
                    }
                }
                Field(other[i].value, "None of these? Type your own answer") { other[i].value = it }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                    Btn("Skip", tone = Tone.Ghost) { model.act { answerQuestion(chat.summary.id, e.id, emptyList()) } }
                    if (i > 0) Btn("Back") { step = i - 1 }
                    Btn(if (last) "Answer" else "Next", tone = Tone.Primary, enabled = ready) {
                        if (last) {
                            val result = e.questions.indices.map { k -> picks[k].toList() + listOfNotNull(other[k].value.trim().ifEmpty { null }) }
                            model.act { answerQuestion(chat.summary.id, e.id, result) }
                        } else step = i + 1
                    }
                }
            }
        }
    }
}

/** Plain text field in a quiet frame. */
@Composable
fun Field(value: String, placeholder: String, singleLine: Boolean = false, modifier: Modifier = Modifier, onChange: (String) -> Unit) {
    Box(modifier.fillMaxWidth().clip(Radius.md).background(pal.surface).border(1.dp, pal.borderStrong, Radius.md).padding(horizontal = 12.dp, vertical = 11.dp)) {
        if (value.isEmpty()) Text(placeholder, style = type.body.copy(color = pal.textFaint))
        BasicTextField(value, onChange, singleLine = singleLine, textStyle = type.body, cursorBrush = SolidColor(pal.brand), modifier = Modifier.fillMaxWidth())
    }
}

// ---------------------------------------------------------------------------------------------
// Composer

@Composable
private fun Composer(model: AppModel, chat: ChatDetail) {
    val id = chat.summary.id
    var text by remember(id) { mutableStateOf("") }
    val insets = WindowInsets.ime.union(WindowInsets.navigationBars)
    Column(Modifier.fillMaxWidth().background(pal.bg).windowInsetsPadding(insets).padding(start = 12.dp, end = 12.dp, top = 6.dp, bottom = 10.dp)) {
        if (!chat.live) {
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        if (chat.summary.status == ChatStatus.ERROR) (chat.summary.error ?: "The agent stopped with an error") else "This chat has ended",
                        style = type.body.copy(color = pal.textMuted),
                        modifier = Modifier.weight(1f),
                    )
                    if (chat.summary.parentId == null) Btn("Resume", tone = Tone.Primary, icon = Res.drawable.ic_play) { model.act { resume(id) } }
                }
            }
            return@Column
        }

        // Queued messages: they send themselves when the turn ends.
        chat.queue.forEach { q ->
            Row(
                Modifier.fillMaxWidth().padding(bottom = 6.dp).clip(Radius.md).background(pal.surfaceHover).padding(start = 12.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Ico(Res.drawable.ic_corner_down_right, size = 14.dp, tint = pal.textFaint)
                Spacer(Modifier.width(8.dp))
                Text(q.text, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Text("Send now", style = type.caption.copy(color = pal.text), modifier = Modifier.tap {
                    model.act { dequeue(id, q.id); send(id, q.text, Delivery.INTERRUPT) }
                }.padding(horizontal = 8.dp, vertical = 8.dp))
                IconBtn(Res.drawable.ic_x, size = 32.dp) { model.act { dequeue(id, q.id) } }
            }
        }

        // Slash commands.
        val slash = text.startsWith("/") && text.none { it.isWhitespace() }
        val matches = if (slash) {
            val q = text.drop(1).lowercase()
            chat.commands.filter { it.name.lowercase().contains(q) }.sortedBy { if (it.name.lowercase().startsWith(q)) 0 else 1 }.take(6)
        } else emptyList()
        AnimatedVisibility(matches.isNotEmpty()) {
            Card(Modifier.fillMaxWidth().padding(bottom = 6.dp), bg = pal.raised, border = pal.borderStrong) {
                Column(Modifier.padding(vertical = 4.dp)) {
                    matches.forEach { c ->
                        Row(
                            Modifier.fillMaxWidth().tap { text = "/${c.name} " }.padding(horizontal = 14.dp, vertical = 9.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text("/${c.name}", style = type.mono.copy(fontWeight = FontWeight.Medium))
                            c.hint?.let { Text(" $it", style = type.mono.copy(color = pal.textFaint)) }
                            Spacer(Modifier.width(10.dp))
                            Text(c.description ?: "", style = type.caption.copy(color = pal.textMuted), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        }
                    }
                }
            }
        }

        val busy = chat.busy
        val hasText = text.isNotBlank()
        Column(Modifier.fillMaxWidth().clip(Radius.xl).background(pal.surface).border(1.dp, pal.borderStrong, Radius.xl)) {
            val input = type.reading.copy(fontSize = 16.5.sp)
            Box(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 6.dp)) {
                if (text.isEmpty()) Text(if (busy) "Queue a message, or steer…" else "Message ${chat.summary.agent.shortName}", style = input.copy(color = pal.textFaint))
                BasicTextField(
                    value = text,
                    onValueChange = { text = it },
                    textStyle = input,
                    cursorBrush = SolidColor(pal.brand),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                    maxLines = 10,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                // Scrolls sideways when the settings don't fit, so the send button never squeezes.
                Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
                    ModeMenu(model, chat)
                    ChatSettings(model, chat)
                }
                if (busy && hasText) {
                    if (chat.canSteer) {
                        Text("Steer", style = type.label, modifier = Modifier.tap(shape = Radius.full) {
                            val t = text; text = ""; model.act { send(id, t, Delivery.STEER) }
                        }.padding(horizontal = 10.dp, vertical = 9.dp))
                    }
                    Text("Send now", style = type.label, modifier = Modifier.tap(shape = Radius.full) {
                        val t = text; text = ""; model.act { send(id, t, Delivery.INTERRUPT) }
                    }.padding(horizontal = 10.dp, vertical = 9.dp))
                }
                if (busy && !hasText) {
                    IconBtn(Res.drawable.ic_square, tint = pal.text, size = 42.dp, bg = pal.surfaceHover) { model.act { interrupt(id) } }
                } else {
                    IconBtn(Res.drawable.ic_arrow_up, tint = if (hasText) pal.bg else pal.textFaint, size = 42.dp, bg = if (hasText) pal.text else pal.surfaceHover) {
                        if (hasText) { val t = text; text = ""; model.act { send(id, t, Delivery.NORMAL) } }
                    }
                }
            }
        }
        if (busy && hasText) {
            Text(
                if (chat.canSteer) "Send queues it for when the agent finishes · Steer adds it to this turn · Send now stops and sends"
                else "Send queues it for when the agent finishes · Send now stops and sends",
                style = type.caption.copy(color = pal.textFaint),
                modifier = Modifier.padding(start = 6.dp, top = 6.dp),
            )
        }
    }
}

@Composable
private fun ModeMenu(model: AppModel, chat: ChatDetail) {
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier.clip(Radius.full).tap(shape = Radius.full) { open = true }.padding(horizontal = 10.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Ico(Res.drawable.ic_shield_alert, size = 15.dp, tint = if (chat.mode.dangerous) pal.danger else pal.textMuted)
            Text(chat.mode.label, style = type.caption.copy(color = if (chat.mode.dangerous) pal.danger else pal.textMuted))
            Ico(Res.drawable.ic_chevron_down, size = 13.dp, tint = pal.textFaint)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = pal.raised) {
            chat.modes.forEach { m ->
                DropdownMenuItem(
                    text = {
                        Column(Modifier.padding(vertical = 4.dp)) {
                            Text(m.label, style = type.label.copy(color = if (m.dangerous) pal.danger else pal.text))
                            Text(m.description, style = type.caption.copy(color = pal.textMuted))
                        }
                    },
                    trailingIcon = { if (m.id == chat.mode.id) Ico(Res.drawable.ic_check, tint = pal.text, size = 16.dp) },
                    onClick = { open = false; model.act { setMode(chat.summary.id, m.id) } },
                )
            }
        }
    }
}

/**
 * The model, reasoning and context of a running chat behind one chip. Reasoning comes from the
 * chat's model, or the agent's default entry when the model isn't one it listed.
 */
@Composable
private fun ChatSettings(model: AppModel, chat: ChatDetail) {
    if (chat.models.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    val id = chat.summary.id
    val name = chat.models.firstOrNull { it.id == chat.model }?.name ?: chat.model ?: "Default model"
    val label = listOfNotNull(
        name,
        chat.efforts.firstOrNull { it.id == chat.effort }?.label,
        chat.contexts.firstOrNull { it.id == chat.contextLimit }?.label,
    ).joinToString(" · ")
    SettingChip(Res.drawable.ic_sparkles, label, maxWidth = 190.dp) { open = true }
    if (open) {
        RunSettingsSheet(
            models = chat.models,
            model = chat.model,
            efforts = chat.efforts,
            effort = chat.effort,
            contexts = chat.contexts,
            context = chat.contextLimit,
            allowDefault = false,
            onDismiss = { open = false },
            onModel = { m -> if (m != null && m != chat.model) model.act { setModel(id, m) } },
            onEffort = { e -> if (e != null && e != chat.effort) model.act { setEffort(id, e) } },
            // The desktop restarts the agent for this and refuses mid-turn; act() shows why.
            onContext = { c -> if (c != null && c != chat.contextLimit) model.act { setContext(id, c) } },
        )
    }
}

@Composable
private fun SettingChip(icon: DrawableResource, label: String, maxWidth: Dp = 110.dp, onClick: () -> Unit) {
    Row(
        Modifier.clip(Radius.full).tap(shape = Radius.full, onClick = onClick).padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Ico(icon, size = 15.dp, tint = pal.textMuted)
        Text(label, style = type.caption.copy(color = pal.textMuted), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = maxWidth))
        Ico(Res.drawable.ic_chevron_down, size = 13.dp, tint = pal.textFaint)
    }
}
