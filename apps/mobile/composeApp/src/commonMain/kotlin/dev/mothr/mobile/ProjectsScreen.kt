package dev.mothr.mobile

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.mothr.client.DirView
import dev.mothr.client.FileEntryView
import dev.mothr.client.FileHitView
import dev.mothr.client.FileView
import dev.mothr.client.ProjectView
import dev.mothr.mobile.resources.Res
import dev.mothr.mobile.resources.ic_arrow_left
import dev.mothr.mobile.resources.ic_book_open
import dev.mothr.mobile.resources.ic_chevron_down
import dev.mothr.mobile.resources.ic_chevron_right
import dev.mothr.mobile.resources.ic_file
import dev.mothr.mobile.resources.ic_file_code
import dev.mothr.mobile.resources.ic_file_search
import dev.mothr.mobile.resources.ic_file_text
import dev.mothr.mobile.resources.ic_folder
import dev.mothr.mobile.resources.ic_folder_git_2
import dev.mothr.mobile.resources.ic_folder_open
import dev.mothr.mobile.resources.ic_git_branch
import dev.mothr.mobile.resources.ic_list_tree
import dev.mothr.mobile.resources.ic_maximize_2
import dev.mothr.mobile.resources.ic_message_square_plus
import dev.mothr.mobile.resources.ic_minimize_2
import dev.mothr.mobile.resources.ic_search
import dev.mothr.mobile.resources.ic_x
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.jetbrains.compose.resources.DrawableResource

// ---------------------------------------------------------------------------------------------
// The list of projects

/** Every project the desktop has indexed, with fuzzy search. */
@Composable
fun ProjectsScreen(model: AppModel) {
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<ProjectView>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(query, model.current) {
        if (query.isNotEmpty()) delay(120) // let typing settle
        model.findProjects(query)
            .onSuccess { hits = it; error = null }
            .onFailure { error = it.message }
    }
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconBtn(Res.drawable.ic_arrow_left, tint = pal.text) { model.projectBack() }
            Column(Modifier.weight(1f).padding(start = 4.dp)) {
                Text("Repos", style = type.display)
                val n = hits?.size
                Text(
                    when {
                        n == null -> "Loading…"
                        query.isNotBlank() -> "$n match${if (n == 1) "" else "es"}"
                        else -> "$n on ${model.current?.name ?: "your computer"}"
                    },
                    style = type.caption.copy(color = pal.textFaint),
                )
            }
        }
        SearchBox(query, "Find a repo by name or path", Res.drawable.ic_search) { query = it }
        val list = hits
        when {
            error != null && list == null -> Notice(error ?: "", pal.danger)
            list == null -> Notice("Loading repos…", pal.textMuted)
            list.isEmpty() -> Notice(if (query.isBlank()) "No repos indexed yet" else "No repos match", pal.textMuted)
            else -> LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp + navBottom())) {
                items(list, key = { it.path }) { p -> ProjectRow(p) { model.openProject(p) } }
            }
        }
    }
}

@Composable
private fun ProjectRow(p: ProjectView, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().tap(shape = Radius.md, onClick = onClick).padding(horizontal = 20.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(
            Modifier.size(42.dp).clip(Radius.md).background(pal.surface).border(1.dp, pal.border, Radius.md),
            contentAlignment = Alignment.Center,
        ) { Ico(Res.drawable.ic_folder_git_2, tint = if (p.pinned) pal.text else pal.textMuted, size = 19.dp) }
        Column(Modifier.weight(1f)) {
            Text(
                highlight(p.name, p.nameIndices, pal.brand),
                style = type.label.copy(fontSize = 16.sp),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(2.dp))
            Text(p.displayPath, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (p.branch != null || p.tags.isNotEmpty()) {
                Spacer(Modifier.height(5.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    p.branch?.let { b ->
                        Ico(Res.drawable.ic_git_branch, tint = pal.textFaint, size = 12.dp)
                        Text(b, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 150.dp))
                    }
                    p.tags.take(3).forEach { Tag(it) }
                }
            }
        }
        Ico(Res.drawable.ic_chevron_right, tint = pal.textFaint, size = 16.dp)
    }
}

// ---------------------------------------------------------------------------------------------
// One project

/**
 * A project's files: folders with their README, and a file viewer. Full screen hides the
 * header so a file gets the whole phone, and the tree button lists the whole project as a tree.
 */
@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Suppress("DEPRECATION")
@Composable
fun ProjectScreen(model: AppModel) {
    val project = model.openProject ?: return
    var fullscreen by remember(project.path) { mutableStateOf(false) }
    var treeOpen by remember(project.path) { mutableStateOf(false) }
    var finding by remember(project.path) { mutableStateOf(false) }
    // Listings stay cached while the project is open; the tree and the folder view share them.
    val dirs = remember(project.path) { mutableStateMapOf<String, Result<DirView>>() }

    androidx.compose.ui.backhandler.BackHandler(enabled = treeOpen || finding || fullscreen) {
        when {
            treeOpen -> treeOpen = false
            finding -> finding = false
            else -> fullscreen = false
        }
    }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
            if (!fullscreen) ProjectHeader(model, project, onFind = { finding = !finding })
            if (finding) {
                FileFinder(model, project) { path ->
                    finding = false
                    model.openFile(path)
                }
            } else {
                val file = model.projectFile
                if (file == null) {
                    FolderView(model, project, model.projectDir, dirs, fullscreen, onFullscreen = { fullscreen = it }, onTree = { treeOpen = true })
                } else {
                    FileScreen(model, project, file, fullscreen, onFullscreen = { fullscreen = it }, onTree = { treeOpen = true })
                }
            }
        }

        // The tree slides in over the content.
        AnimatedVisibility(treeOpen, enter = fadeIn(tween(160)), exit = fadeOut(tween(160))) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)).tap(shape = Radius.sm) { treeOpen = false })
        }
        AnimatedVisibility(
            treeOpen,
            enter = slideInHorizontally(tween(240)) { -it },
            exit = slideOutHorizontally(tween(200)) { -it },
        ) {
            TreePanel(model, project, dirs, onClose = { treeOpen = false }) { entry ->
                treeOpen = false
                if (entry.dir) model.browse(entry.path) else model.openFile(entry.path)
            }
        }
    }
}

@Composable
private fun ProjectHeader(model: AppModel, project: ProjectView, onFind: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, end = 8.dp, top = 4.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconBtn(Res.drawable.ic_arrow_left, tint = pal.text) { model.projectBack() }
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            Text(project.name, style = type.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                Text(project.displayPath, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                project.branch?.let {
                    Ico(Res.drawable.ic_git_branch, tint = pal.textFaint, size = 12.dp)
                    Text(it, style = type.caption.copy(color = pal.textFaint), maxLines = 1)
                }
            }
        }
        IconBtn(Res.drawable.ic_file_search) { onFind() }
        IconBtn(Res.drawable.ic_message_square_plus) { model.newChatIn(project) }
    }
    Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
}

/** project / src / ui — tap a part to open that folder. */
@Composable
private fun Crumbs(model: AppModel, project: ProjectView, path: String, isFile: Boolean, modifier: Modifier = Modifier) {
    val parts = path.split('/').filter { it.isNotEmpty() }
    Row(modifier.horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
        Crumb(project.name, last = parts.isEmpty()) { model.browse("") }
        parts.forEachIndexed { i, part ->
            Text("/", style = type.body.copy(color = pal.textFaint), modifier = Modifier.padding(horizontal = 2.dp))
            val target = parts.take(i + 1).joinToString("/")
            val last = i == parts.lastIndex
            Crumb(part, last = last) { if (!(last && isFile)) model.browse(target) }
        }
    }
}

@Composable
private fun Crumb(label: String, last: Boolean, onClick: () -> Unit) {
    Text(
        label,
        style = type.label.copy(color = if (last) pal.text else pal.textMuted, fontWeight = if (last) FontWeight.SemiBold else FontWeight.Medium),
        maxLines = 1,
        modifier = Modifier.clip(Radius.sm).tap(enabled = !last, shape = Radius.sm, onClick = onClick).padding(horizontal = 6.dp, vertical = 6.dp),
    )
}

/** Load a folder into the shared cache once. */
@Composable
private fun rememberDir(model: AppModel, project: ProjectView, dir: String, dirs: SnapshotStateMap<String, Result<DirView>>): Result<DirView>? {
    LaunchedEffect(project.path, dir) {
        if (dirs[dir]?.isSuccess != true) dirs[dir] = model.listDir(project, dir)
    }
    return dirs[dir]
}

@Composable
private fun FolderView(
    model: AppModel,
    project: ProjectView,
    dir: String,
    dirs: SnapshotStateMap<String, Result<DirView>>,
    fullscreen: Boolean,
    onFullscreen: (Boolean) -> Unit,
    onTree: () -> Unit,
) {
    val listing = rememberDir(model, project, dir, dirs)
    var readme by remember(project.path, dir) { mutableStateOf<FileView?>(null) }
    val readmePath = listing?.getOrNull()?.readme
    LaunchedEffect(readmePath) {
        readme = readmePath?.let { model.readFile(project, it).getOrNull() }
    }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 10.dp, end = 6.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Crumbs(model, project, dir, isFile = false, modifier = Modifier.weight(1f))
            IconBtn(Res.drawable.ic_list_tree, size = 36.dp) { onTree() }
            IconBtn(if (fullscreen) Res.drawable.ic_minimize_2 else Res.drawable.ic_maximize_2, size = 36.dp) { onFullscreen(!fullscreen) }
        }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 6.dp, bottom = 32.dp + navBottom())) {
            when {
                listing == null -> item { Notice("Loading…", pal.textMuted) }
                listing.isFailure -> item { Notice(listing.exceptionOrNull()?.message ?: "Can't open this folder", pal.danger) }
                else -> {
                    val l = listing.getOrThrow()
                    if (l.entries.isEmpty()) item { Notice("This folder is empty", pal.textMuted) }
                    itemsIndexed(l.entries, key = { _, e -> e.path }) { i, e ->
                        EntryRow(e, first = i == 0, last = i == l.entries.lastIndex) {
                            if (e.dir) model.browse(e.path) else model.openFile(e.path)
                        }
                    }
                    if (l.truncated) item { Text("Only the first entries are listed. Use search to find the rest.", style = type.caption.copy(color = pal.textFaint), modifier = Modifier.padding(12.dp)) }
                    readme?.let { r ->
                        item(key = "readme") {
                            Column(Modifier.padding(top = 20.dp).fillMaxWidth().clip(Radius.lg).background(pal.surface).border(1.dp, pal.border, Radius.lg)) {
                                Row(
                                    Modifier.fillMaxWidth().background(pal.chrome).tap(shape = Radius.sm) { model.openFile(r.path) }.padding(horizontal = 14.dp, vertical = 11.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Ico(Res.drawable.ic_book_open, size = 15.dp)
                                    Text(r.name, style = type.label, modifier = Modifier.weight(1f))
                                    Text("Open", style = type.caption.copy(color = pal.textFaint))
                                }
                                Box(Modifier.padding(16.dp)) {
                                    val text = r.text ?: ""
                                    if (r.markdown) DocLinks(model, project, r.path) { MarkdownText(text) }
                                    else Text(text, style = type.mono)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** One row of a folder, as part of a card (rounded at the ends). */
@Composable
private fun EntryRow(e: FileEntryView, first: Boolean, last: Boolean, onClick: () -> Unit) {
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(
        topStart = if (first) 14.dp else 0.dp, topEnd = if (first) 14.dp else 0.dp,
        bottomStart = if (last) 14.dp else 0.dp, bottomEnd = if (last) 14.dp else 0.dp,
    )
    Column(Modifier.fillMaxWidth().clip(shape).background(pal.surface)) {
        if (!first) Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
        Row(
            Modifier.fillMaxWidth().tap(shape = Radius.sm, onClick = onClick).padding(horizontal = 14.dp, vertical = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Ico(entryIcon(e.name, e.dir), tint = if (e.dir) pal.textMuted else pal.textFaint, size = 17.dp)
            Text(e.name, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            if (e.dir) Ico(Res.drawable.ic_chevron_right, tint = pal.textFaint, size = 15.dp)
            else Text(e.sizeLabel, style = type.caption.copy(color = pal.textFaint))
        }
    }
}

@Composable
private fun FileScreen(
    model: AppModel,
    project: ProjectView,
    path: String,
    fullscreen: Boolean,
    onFullscreen: (Boolean) -> Unit,
    onTree: () -> Unit,
) {
    var file by remember(project.path, path) { mutableStateOf<Result<FileView>?>(null) }
    var source by remember(project.path, path) { mutableStateOf(false) }
    LaunchedEffect(project.path, path) {
        val r = model.readFile(project, path)
        // A link may name a folder; show it as one.
        if (r.exceptionOrNull()?.message?.contains("is a folder") == true) model.browse(path) else file = r
    }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 6.dp, top = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            if (fullscreen) IconBtn(Res.drawable.ic_arrow_left, tint = pal.text, size = 36.dp) { model.projectBack() }
            Crumbs(model, project, path, isFile = true, modifier = Modifier.weight(1f))
            IconBtn(Res.drawable.ic_list_tree, size = 36.dp) { onTree() }
            IconBtn(if (fullscreen) Res.drawable.ic_minimize_2 else Res.drawable.ic_maximize_2, size = 36.dp) { onFullscreen(!fullscreen) }
        }
        val f = file?.getOrNull()
        if (f != null) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    listOfNotNull(
                        if (f.text != null) "${f.lineCount} lines" else null,
                        f.sizeLabel,
                        f.language.takeIf { it != "text" },
                        if (f.truncated) "first 1 MB shown" else null,
                    ).joinToString(" · "),
                    style = type.caption.copy(color = pal.textFaint),
                    modifier = Modifier.weight(1f),
                )
                if (f.markdown) {
                    Row(Modifier.clip(Radius.full).background(pal.surfaceHover).padding(3.dp)) {
                        Segment("Preview", !source) { source = false }
                        Segment("Source", source) { source = true }
                    }
                }
            }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
        val r = file
        when {
            r == null -> Notice("Opening…", pal.textMuted)
            r.isFailure -> Notice(r.exceptionOrNull()?.message ?: "Can't open this file", pal.danger)
            f?.text == null -> Notice("Binary file · ${f?.sizeLabel ?: ""} · not shown", pal.textMuted)
            f.markdown && !source -> Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 18.dp, vertical = 16.dp)) {
                DocLinks(model, project, f.path) { MarkdownText(f.text ?: "") }
                Spacer(Modifier.height(40.dp + navBottom()))
            }
            else -> CodeView(f.text ?: "")
        }
    }
}

@Composable
private fun Segment(label: String, on: Boolean, onClick: () -> Unit) {
    Text(
        label,
        style = type.caption.copy(color = if (on) pal.text else pal.textMuted),
        modifier = Modifier.clip(Radius.full).background(if (on) pal.raised else Color.Transparent).tap(shape = Radius.full, onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 5.dp),
    )
}

/** Source with line numbers; long lines scroll sideways together. */
@Composable
private fun CodeView(text: String) {
    val lines = remember(text) { text.lines().let { if (it.size > 1 && it.last().isEmpty()) it.dropLast(1) else it } }
    val gutter = (lines.size.toString().length * 9 + 14).dp
    val h = rememberScrollState()
    LazyColumn(
        Modifier.fillMaxSize().background(pal.codeBg),
        contentPadding = PaddingValues(top = 10.dp, bottom = 32.dp + navBottom()),
    ) {
        itemsIndexed(lines) { i, line ->
            Row(Modifier.fillMaxWidth()) {
                Text(
                    "${i + 1}",
                    style = type.mono.copy(color = pal.textFaint, fontSize = 12.sp),
                    textAlign = TextAlign.End,
                    modifier = Modifier.width(gutter).padding(end = 10.dp),
                )
                Text(
                    line.ifEmpty { " " },
                    style = type.mono.copy(fontSize = 12.5.sp),
                    softWrap = false,
                    modifier = Modifier.horizontalScroll(h).padding(end = 24.dp),
                )
            }
        }
    }
}

/** Links in a document open inside the project; web links open in the browser. */
@Composable
private fun DocLinks(model: AppModel, project: ProjectView, from: String, content: @Composable () -> Unit) {
    val outer = LocalUriHandler.current
    val handler = remember(project.path, from) {
        object : UriHandler {
            override fun openUri(uri: String) {
                when (val target = resolveLink(uri, from)) {
                    null -> {}
                    is LinkTarget.External -> runCatching { outer.openUri(target.url) }.onFailure { model.toast = "Can't open $uri" }
                    is LinkTarget.InProject -> if (target.path.isEmpty()) model.browse("") else model.openFile(target.path)
                    LinkTarget.Outside -> model.toast = "That link points outside the repo"
                }
            }
        }
    }
    CompositionLocalProvider(LocalUriHandler provides handler) { content() }
}

sealed interface LinkTarget {
    data class External(val url: String) : LinkTarget
    data class InProject(val path: String) : LinkTarget
    data object Outside : LinkTarget
}

/** Where a link in the document at [from] leads; null for same-page anchors. */
fun resolveLink(url: String, from: String): LinkTarget? {
    val u = url.trim()
    if (u.startsWith("#")) return null
    val scheme = u.substringBefore(':', "")
    if (scheme.length > 1 && scheme.all { it.isLetterOrDigit() || it in "+-." }) return LinkTarget.External(u)
    val path = u.substringBefore('#').substringBefore('?').replace("%20", " ")
    val parts = if (path.startsWith("/")) mutableListOf() else from.substringBeforeLast('/', "").split('/').filter { it.isNotEmpty() }.toMutableList()
    for (seg in path.split('/')) {
        when (seg) {
            "", "." -> {}
            ".." -> if (parts.isEmpty()) return LinkTarget.Outside else parts.removeAt(parts.lastIndex)
            else -> parts.add(seg)
        }
    }
    return LinkTarget.InProject(parts.joinToString("/"))
}

// ---------------------------------------------------------------------------------------------
// Tree and finder

/** The whole project as a tree; folders open in place. */
@Composable
private fun TreePanel(
    model: AppModel,
    project: ProjectView,
    dirs: SnapshotStateMap<String, Result<DirView>>,
    onClose: () -> Unit,
    onPick: (FileEntryView) -> Unit,
) {
    val current = model.projectFile ?: model.projectDir
    // Open the folders down to what's on screen.
    val expanded = remember(project.path) {
        mutableStateListOf<String>().apply {
            var acc = ""
            current.split('/').filter { it.isNotEmpty() }.dropLast(if (model.projectFile != null) 1 else 0).forEach {
                acc = if (acc.isEmpty()) it else "$acc/$it"
                add(acc)
            }
        }
    }
    val scope = rememberCoroutineScope()
    fun ensure(dir: String) {
        if (dirs[dir]?.isSuccess != true) scope.launch { dirs[dir] = model.listDir(project, dir) }
    }
    LaunchedEffect(project.path) {
        ensure("")
        expanded.forEach { ensure(it) }
    }
    val rows = buildList {
        fun walk(dir: String, depth: Int) {
            dirs[dir]?.getOrNull()?.entries?.forEach { e ->
                add(e to depth)
                if (e.dir && e.path in expanded) walk(e.path, depth + 1)
            }
        }
        walk("", 0)
    }
    Column(
        Modifier.fillMaxHeight().fillMaxWidth(0.86f).background(pal.chrome).border(1.dp, pal.border)
            .windowInsetsPadding(WindowInsets.statusBars),
    ) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Ico(Res.drawable.ic_folder_git_2, tint = pal.textMuted, size = 17.dp)
            Spacer(Modifier.width(10.dp))
            Text(project.name, style = type.heading, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).tap(shape = Radius.sm) { model.browse(""); onClose() })
            IconBtn(Res.drawable.ic_x) { onClose() }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(pal.border))
        if (dirs[""] == null) Notice("Loading…", pal.textMuted)
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 6.dp, bottom = 24.dp + navBottom())) {
            items(rows, key = { it.first.path }) { (e, depth) ->
                val open = e.dir && e.path in expanded
                val active = e.path == current
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 6.dp).clip(Radius.sm).background(if (active) pal.brandSoft else Color.Transparent)
                        .tap(shape = Radius.sm) {
                            if (e.dir) {
                                if (open) expanded.remove(e.path) else { expanded.add(e.path); ensure(e.path) }
                            } else onPick(e)
                        }
                        .padding(start = (10 + depth * 16).dp, end = 8.dp, top = 9.dp, bottom = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(7.dp),
                ) {
                    Box(Modifier.width(14.dp)) {
                        if (e.dir) Ico(if (open) Res.drawable.ic_chevron_down else Res.drawable.ic_chevron_right, tint = pal.textFaint, size = 14.dp)
                    }
                    Ico(if (e.dir && open) Res.drawable.ic_folder_open else entryIcon(e.name, e.dir), tint = if (e.dir) pal.textMuted else pal.textFaint, size = 16.dp)
                    Text(
                        e.name,
                        style = type.body.copy(fontSize = 14.sp, fontWeight = if (active) FontWeight.Medium else FontWeight.Normal),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (e.dir && open) {
                        Text("Open", style = type.caption.copy(color = pal.textFaint), modifier = Modifier.clip(Radius.sm).tap(shape = Radius.sm) { onPick(e) }.padding(horizontal = 6.dp, vertical = 2.dp))
                    }
                }
            }
        }
    }
}

/** Fuzzy "go to file" over the whole project. */
@Composable
private fun FileFinder(model: AppModel, project: ProjectView, onPick: (String) -> Unit) {
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<FileHitView>>(emptyList()) }
    LaunchedEffect(query) {
        delay(120)
        hits = if (query.isBlank()) emptyList() else model.findFiles(project, query)
    }
    Column(Modifier.fillMaxSize()) {
        SearchBox(query, "Go to file", Res.drawable.ic_file_search, autoFocus = true) { query = it }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp + navBottom())) {
            if (query.isNotBlank() && hits.isEmpty()) item { Notice("No files match", pal.textMuted) }
            items(hits, key = { it.path }) { h ->
                Row(
                    Modifier.fillMaxWidth().tap(shape = Radius.sm) { onPick(h.path) }.padding(horizontal = 20.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Ico(entryIcon(h.name, false), tint = pal.textFaint, size = 17.dp)
                    Column(Modifier.weight(1f)) {
                        Text(highlight(h.name, h.nameIndices, pal.brand), style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (h.folder.isNotEmpty()) Text(h.folder, style = type.caption.copy(color = pal.textFaint), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Small parts

@Composable
private fun SearchBox(value: String, placeholder: String, icon: DrawableResource, autoFocus: Boolean = false, onChange: (String) -> Unit) {
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    if (autoFocus) LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    Row(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(Radius.lg).background(pal.surfaceHover)
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Ico(icon, tint = pal.textFaint, size = 17.dp)
        Spacer(Modifier.width(10.dp))
        Box(Modifier.weight(1f)) {
            if (value.isEmpty()) Text(placeholder, style = type.body.copy(color = pal.textFaint))
            BasicTextField(
                value = value,
                onValueChange = onChange,
                singleLine = true,
                textStyle = type.body,
                cursorBrush = SolidColor(pal.brand),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )
        }
        if (value.isNotEmpty()) Ico(Res.drawable.ic_x, size = 16.dp, modifier = Modifier.tap { onChange("") })
    }
}

@Composable
private fun Notice(text: String, color: Color) {
    Text(text, style = type.body.copy(color = color), textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(32.dp))
}

@Composable
private fun Tag(text: String) {
    Text(text, style = type.caption.copy(color = pal.textMuted, fontSize = 11.sp), modifier = Modifier.clip(Radius.sm).background(pal.surfaceHover).padding(horizontal = 6.dp, vertical = 1.dp))
}

/** [text] with the matched characters in [color]. */
fun highlight(text: String, indices: List<UInt>, color: Color): AnnotatedString = buildAnnotatedString {
    val hit = indices.map { it.toInt() }.toSet()
    text.forEachIndexed { i, c ->
        if (i in hit) withStyle(SpanStyle(color = color, fontWeight = FontWeight.SemiBold)) { append(c) } else append(c)
    }
}

private fun entryIcon(name: String, dir: Boolean): DrawableResource {
    if (dir) return Res.drawable.ic_folder
    val lower = name.lowercase()
    val ext = lower.substringAfterLast('.', "")
    return when {
        lower.startsWith("readme") -> Res.drawable.ic_book_open
        ext in setOf("md", "mdx", "markdown", "txt") -> Res.drawable.ic_file_text
        ext in CODE_EXTS -> Res.drawable.ic_file_code
        else -> Res.drawable.ic_file
    }
}

private val CODE_EXTS = setOf(
    "rs", "kt", "kts", "java", "swift", "js", "mjs", "ts", "tsx", "jsx", "py", "go", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php",
    "sh", "bash", "zsh", "fish", "html", "css", "scss", "json", "toml", "yaml", "yml", "xml", "sql", "lua", "zig", "dart", "vue", "svelte",
    "gradle", "proto", "graphql",
)
