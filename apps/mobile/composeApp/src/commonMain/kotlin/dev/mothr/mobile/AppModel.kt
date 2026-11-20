package dev.mothr.mobile

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.russhwolf.settings.Settings
import dev.mothr.client.AgentView
import dev.mothr.client.Attention
import dev.mothr.client.AttentionKind
import dev.mothr.client.ChatDetail
import dev.mothr.client.ChatGroup
import dev.mothr.client.ClientException
import dev.mothr.client.ConnectionInfo
import dev.mothr.client.ConnectionState
import dev.mothr.client.DirView
import dev.mothr.client.FileHitView
import dev.mothr.client.FileView
import dev.mothr.client.MothrClient
import dev.mothr.client.ProjectView
import dev.mothr.client.UsageView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.IO
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.random.Random

/** One paired desktop. It stays connected while paired, whichever computer is on screen. */
class Computer internal constructor(
    val id: String,
    client: MothrClient,
    customName: String?,
    hostName: String?,
) {
    var client: MothrClient by mutableStateOf(client)
        internal set
    var connection: ConnectionInfo? by mutableStateOf(null)
        internal set

    /** The name given on this phone. */
    var customName: String? by mutableStateOf(customName)
        internal set

    /** The name the desktop reports; remembered so it still shows while offline. */
    var hostName: String? by mutableStateOf(hostName)
        internal set

    val address: String get() = client.server().removePrefix("https://").removePrefix("http://")
    val name: String get() = customName ?: hostName ?: address
    val needsYou: Int get() = connection?.needsYou?.toInt() ?: 0

    internal var loop: Job? = null

    /** Requests already seen (on screen or as a notification); each notifies at most once. */
    internal val seen = mutableSetOf<String>()

    /** Requests with a notification up. */
    internal val shown = mutableSetOf<String>()

    /** Requests are numbered per desktop, so notifications need the computer in their key. */
    internal fun notificationKey(key: String) = "$id/$key"
}

/**
 * The app's state. All data comes from the Rust client (the desktop's own session model and
 * reducer); this class only turns its change notifications into Compose state.
 *
 * One instance per process ([Mothr.model]). Every paired computer keeps its connection, and any
 * of them can notify; the chat screens show the [current] one. While the app is off screen the
 * connections stay up (if [runInBackground]).
 */
class AppModel(private val scope: CoroutineScope) {
    private val prefs = Settings()

    var computers: List<Computer> by mutableStateOf(emptyList())
        private set
    var current: Computer? by mutableStateOf(null)
        private set
    val client: MothrClient? get() = current?.client
    val connection: ConnectionInfo? get() = current?.connection

    var groups: List<ChatGroup> by mutableStateOf(emptyList())
        private set
    var usage: List<UsageView> by mutableStateOf(emptyList())
        private set
    var agents: List<AgentView> by mutableStateOf(emptyList())
        private set
    var projects: List<ProjectView> by mutableStateOf(emptyList())
        private set

    /** Chat stack: the last id is on screen (subagents push on top of their orchestrator). */
    var stack: List<String> by mutableStateOf(emptyList())
        private set
    var openChat: ChatDetail? by mutableStateOf(null)
        private set

    var query by mutableStateOf("")
    var pairError: String? by mutableStateOf(null)
        private set
    var toast: String? by mutableStateOf(null)
    var showNewChat by mutableStateOf(false)
    var showUsage by mutableStateOf(false)
    var showComputers by mutableStateOf(false)

    /** The unsent new chat, kept if the screen is closed by accident. */
    var newChatDraft by mutableStateOf("")
    var creatingChat by mutableStateOf(false)
        private set

    /** The Projects list is open. */
    var showProjects by mutableStateOf(false)

    /** The project being browsed, the folder on screen ("" is its root) and the open file. */
    var openProject: ProjectView? by mutableStateOf(null)
        private set
    var projectDir by mutableStateOf("")
        private set
    var projectFile: String? by mutableStateOf(null)
        private set

    /** The project a "New chat here" starts in; taken once by the new-chat screen. */
    private var newChatProject: ProjectView? = null

    /** The pairing screen is open on top of an already paired phone. */
    var addingComputer by mutableStateOf(false)
        private set

    /** Keep the connections and notifications going while the app is closed. */
    var runInBackground: Boolean by mutableStateOf(prefs.getBoolean(KEY_BACKGROUND, true))
        private set

    private val background get() = Mothr.background

    /** The UI is on screen. Off screen, only the connection and [Attention] are tracked. */
    private var foreground = false

    /** Disconnected from the notification; reconnects when the app is opened. */
    private var paused = false
    private var lastStatus = ""

    init {
        restore()
    }

    private fun restore() {
        // Before several computers were supported there was a single saved link.
        prefs.getStringOrNull(KEY_LINK)?.let { link ->
            prefs.putString(linkKey(newId().also { saveIds(listOf(it)) }), link)
            prefs.remove(KEY_LINK)
        }
        val saved = savedIds().mapNotNull { id ->
            val link = prefs.getStringOrNull(linkKey(id)) ?: return@mapNotNull null
            val client = try {
                MothrClient(link)
            } catch (e: Throwable) {
                return@mapNotNull null
            }
            Computer(id, client, prefs.getStringOrNull(nameKey(id)), prefs.getStringOrNull(hostKey(id)))
        }
        if (saved.isEmpty()) return
        computers = saved
        current = saved.firstOrNull { it.id == prefs.getStringOrNull(KEY_CURRENT) } ?: saved.first()
        saved.forEach(::start)
        background.keepAlive(runInBackground)
    }

    private fun savedIds() = prefs.getString(KEY_COMPUTERS, "").split(',').filter { it.isNotEmpty() }
    private fun saveIds(ids: List<String>) = prefs.putString(KEY_COMPUTERS, ids.joinToString(","))
    private fun newId() = Random.nextLong().toULong().toString(16)

    private fun start(computer: Computer) {
        computer.client.connect()
        startLoop(computer)
    }

    /** Called by the platform when the UI appears or goes away. */
    fun setForeground(on: Boolean) {
        if (foreground == on) return
        foreground = on
        if (computers.isEmpty()) return
        if (on) {
            resume()
            computers.forEach { c -> scope.launch { refresh(c) } }
        } else if (!runInBackground) {
            disconnectAll()
            paused = true
        }
    }

    private fun resume() {
        if (!paused) return
        paused = false
        computers.forEach { it.client.connect() }
        background.keepAlive(runInBackground)
    }

    private fun disconnectAll() {
        val clients = computers.map { it.client }
        scope.launch(Dispatchers.IO) { clients.forEach { it.disconnect() } }
    }

    fun toggleBackground(on: Boolean) {
        runInBackground = on
        prefs.putBoolean(KEY_BACKGROUND, on)
        if (computers.isNotEmpty() && !paused) background.keepAlive(on)
    }

    /** "Disconnect" on the ongoing notification: stop until the app is opened again. */
    fun pauseUntilOpened() {
        if (computers.isEmpty()) return
        paused = true
        background.keepAlive(false)
        disconnectAll()
        computers.forEach(::clearNotifications)
    }

    /** Open a chat from a notification: [path] is the orchestrator then the subagent. */
    fun openPath(computerId: String, path: List<String>) {
        val computer = computers.firstOrNull { it.id == computerId } ?: return
        if (path.isEmpty()) return
        select(computer)
        addingComputer = false
        showNewChat = false
        showUsage = false
        stack = path
        openChat = computer.client.chat(path.last())
        withdrawChat(computer, path.last())
        act { markRead(path.last()) }
    }

    /**
     * Answer from a notification without opening the app. Returns an error message, or null
     * when the desktop took the answer. The answer only ever goes to the computer that asked.
     */
    suspend fun quickReply(computerId: String, chatId: String, entryId: String, kind: AttentionKind, replyId: String): String? {
        val computer = computers.firstOrNull { it.id == computerId }
            ?: return "That computer isn't paired anymore".also { background.failed(it) }
        val c = computer.client
        return try {
            withContext(Dispatchers.IO) {
                when (kind) {
                    AttentionKind.PERMISSION -> c.respondPermission(chatId, entryId, replyId)
                    AttentionKind.QUESTION -> c.answerQuestion(chatId, entryId, listOf(listOf(replyId)))
                }
            }
            val key = "$chatId/$entryId"
            if (computer.shown.remove(key)) background.withdraw(computer.notificationKey(key))
            null
        } catch (e: Throwable) {
            e.friendly().also { background.failed(it) }
        }
    }

    /**
     * One background check (iOS background refresh): connect, wait for each desktop's state,
     * post notifications, and drop the connections again if the app is still off screen.
     * Computers are checked side by side, so an unreachable one can't use up the time.
     */
    suspend fun checkOnce(timeoutMs: Long) {
        coroutineScope {
            computers.forEach { computer ->
                launch {
                    val c = computer.client
                    c.connect()
                    var rev = 0uL
                    val start = kotlin.time.TimeSource.Monotonic.markNow()
                    while (start.elapsedNow().inWholeMilliseconds < timeoutMs) {
                        val conn = c.connection()
                        if (conn.loaded && conn.state == ConnectionState.CONNECTED) break
                        if (conn.state == ConnectionState.UNAUTHORIZED) break
                        rev = withContext(Dispatchers.IO) { c.waitForChange(rev, 1_000u) }
                    }
                    refresh(computer)
                    if (!foreground && !runInBackground) withContext(Dispatchers.IO) { c.disconnect() }
                }
            }
        }
    }

    fun startAdding() {
        pairError = null
        showComputers = false
        addingComputer = true
    }

    fun cancelAdding() {
        pairError = null
        addingComputer = false
    }

    /**
     * Pair from a link. A link for a computer that's already here (same token, or same address
     * with a new token) updates that entry, so re-pairing never creates a duplicate.
     */
    fun pair(link: String) {
        pairError = null
        val client = try {
            MothrClient(link)
        } catch (e: Throwable) {
            pairError = e.friendly()
            return
        }
        val existing = computers.firstOrNull { token(it.client) == token(client) || it.client.server() == client.server() }
        val computer = when {
            existing == null -> Computer(newId(), client, null, null).also {
                computers = computers + it
                saveIds(computers.map { c -> c.id })
                start(it)
            }
            existing.client.link() == client.link() -> {
                client.close()
                toast = "Already paired with ${existing.name}"
                existing
            }
            else -> existing.also { replaceClient(it, client) }
        }
        prefs.putString(linkKey(computer.id), computer.client.link())
        addingComputer = false
        select(computer)
        resume()
        background.keepAlive(runInBackground)
    }

    private fun token(client: MothrClient) = client.link().substringAfter("?token=")

    private fun replaceClient(computer: Computer, client: MothrClient) {
        shutDown(computer)
        computer.client = client
        computer.connection = null
        start(computer)
    }

    /** Stop a computer's connection and take down what it had on the phone. */
    private fun shutDown(computer: Computer) {
        computer.loop?.cancel()
        clearNotifications(computer)
        computer.seen.clear()
        val old = computer.client
        scope.launch(Dispatchers.IO) { old.disconnect(); old.close() }
    }

    /** Show this computer's chats. */
    fun select(computer: Computer) {
        showComputers = false
        if (current === computer) return
        current = computer
        prefs.putString(KEY_CURRENT, computer.id)
        clearView()
        scope.launch { refresh(computer) }
    }

    private fun clearView() {
        stack = emptyList()
        openChat = null
        groups = emptyList()
        usage = emptyList()
        agents = emptyList()
        projects = emptyList()
        query = ""
        showNewChat = false
        showUsage = false
        showProjects = false
        closeProject()
    }

    /** Give a computer a name on this phone; a blank name goes back to the desktop's own. */
    fun rename(computer: Computer, name: String) {
        val clean = name.trim().ifEmpty { null }
        computer.customName = clean
        if (clean != null) prefs.putString(nameKey(computer.id), clean) else prefs.remove(nameKey(computer.id))
        updateStatus()
    }

    fun unpair(computer: Computer) {
        shutDown(computer)
        computers = computers - computer
        saveIds(computers.map { it.id })
        listOf(linkKey(computer.id), nameKey(computer.id), hostKey(computer.id), projectKey(computer.id)).forEach(prefs::remove)
        if (current === computer) {
            current = computers.firstOrNull()
            clearView()
            val next = current
            if (next != null) {
                prefs.putString(KEY_CURRENT, next.id)
                scope.launch { refresh(next) }
            } else {
                prefs.remove(KEY_CURRENT)
            }
        }
        if (computers.isEmpty()) {
            showComputers = false
            addingComputer = false
            background.keepAlive(false)
        }
        updateStatus()
    }

    private fun startLoop(computer: Computer) {
        computer.loop?.cancel()
        val c = computer.client
        computer.loop = scope.launch {
            var rev = 0uL
            refresh(computer)
            while (isActive) {
                rev = withContext(Dispatchers.IO) { c.waitForChange(rev, 15_000u) }
                refresh(computer)
                // Streaming produces many tiny changes; ~30 updates/s is plenty.
                delay(33)
            }
        }
    }

    private suspend fun refresh(computer: Computer) {
        val c = computer.client
        if (!foreground || computer !== current) {
            // Not on screen: just the connection and what needs the user.
            val (conn, items) = withContext(Dispatchers.Default) { c.connection() to c.attention() }
            if (computer.client === c) update(computer, conn, items)
            return
        }
        val id = stack.lastOrNull()
        val q = query
        val snapshot = withContext(Dispatchers.Default) {
            Snapshot(c.connection(), c.chatGroups(q), c.usage(), c.agents(), c.projects(), id?.let { c.chat(it) }, c.takeToasts(), c.attention())
        }
        if (computer.client !== c) return
        update(computer, snapshot.connection, snapshot.attention)
        // The user may have switched computers while this was loading.
        if (computer !== current) return
        groups = snapshot.groups
        usage = snapshot.usage
        agents = snapshot.agents
        projects = snapshot.projects
        if (id == stack.lastOrNull()) openChat = snapshot.chat
        snapshot.toasts.lastOrNull()?.let { toast = it.text }
    }

    private data class Snapshot(
        val connection: ConnectionInfo,
        val groups: List<ChatGroup>,
        val usage: List<UsageView>,
        val agents: List<AgentView>,
        val projects: List<ProjectView>,
        val chat: ChatDetail?,
        val toasts: List<dev.mothr.client.Toast>,
        val attention: List<Attention>,
    )

    private fun update(computer: Computer, conn: ConnectionInfo, items: List<Attention>) {
        computer.connection = conn
        conn.computer?.takeIf { it != computer.hostName }?.let {
            computer.hostName = it
            prefs.putString(hostKey(computer.id), it)
        }
        track(computer, conn, items)
        updateStatus()
    }

    /** Notify about new requests that aren't on screen, and withdraw answered ones. */
    private fun track(computer: Computer, conn: ConnectionInfo, items: List<Attention>) {
        // The list is only meaningful once the desktop's snapshot is in.
        if (!conn.loaded) return
        val onScreen = foreground && computer === current
        val keys = items.mapTo(HashSet()) { it.key }
        for (a in items) {
            if (computer.seen.add(a.key) && !onScreen) {
                val source = NotificationSource(computer.id, computer.name.takeIf { computers.size > 1 })
                background.show(a.copy(key = computer.notificationKey(a.key)), source)
                computer.shown += a.key
            }
        }
        computer.seen.retainAll(keys)
        (computer.shown - keys).forEach {
            computer.shown -= it
            background.withdraw(computer.notificationKey(it))
        }
    }

    /** The one line on the ongoing "connected" notification. */
    private fun updateStatus() {
        val waiting = computers.sumOf { it.needsYou }
        val connected = computers.filter { it.connection?.state == ConnectionState.CONNECTED }
        val only = computers.singleOrNull()
        val text = when {
            computers.isEmpty() -> "Disconnected"
            waiting == 1 -> "1 chat needs you"
            waiting > 1 -> "$waiting chats need you"
            only != null -> when (only.connection?.state) {
                ConnectionState.CONNECTED -> "Connected to ${only.name}"
                ConnectionState.UNAUTHORIZED -> "Not paired anymore. Open Mothr to pair again."
                ConnectionState.IDLE -> "Disconnected"
                else -> "Reconnecting…"
            }
            connected.size == computers.size -> "Connected to ${computers.size} computers"
            connected.isEmpty() -> "Reconnecting…"
            else -> "Connected to ${connected.size} of ${computers.size} computers"
        }
        if (text != lastStatus) {
            lastStatus = text
            background.status(text)
        }
    }

    private fun withdrawChat(computer: Computer, chatId: String) {
        computer.shown.filter { it.startsWith("$chatId/") }.forEach {
            computer.shown -= it
            background.withdraw(computer.notificationKey(it))
        }
    }

    private fun clearNotifications(computer: Computer) {
        computer.shown.forEach { background.withdraw(computer.notificationKey(it)) }
        computer.shown.clear()
    }

    fun search(q: String) {
        query = q
        val computer = current ?: return
        scope.launch {
            val found = withContext(Dispatchers.Default) { computer.client.chatGroups(q) }
            if (computer === current) groups = found
        }
    }

    fun open(id: String) {
        val computer = current ?: return
        stack = stack + id
        openChat = computer.client.chat(id)
        withdrawChat(computer, id)
        act { markRead(id) }
    }

    fun back() {
        stack = stack.dropLast(1)
        openChat = stack.lastOrNull()?.let { client?.chat(it) }
    }

    /** Run a client command off the main thread; failures become a toast. */
    fun act(onDone: (() -> Unit)? = null, block: MothrClient.() -> Unit) {
        val c = client ?: return
        scope.launch {
            try {
                withContext(Dispatchers.IO) { c.block() }
                onDone?.invoke()
            } catch (e: Throwable) {
                toast = e.friendly()
            }
        }
    }

    suspend fun searchProjects(q: String): List<ProjectView> {
        val c = client ?: return emptyList()
        return try {
            withContext(Dispatchers.IO) { c.searchProjects(q) }
        } catch (e: Throwable) {
            toast = e.friendly()
            emptyList()
        }
    }

    // ---------------------------------------------------------------------------------------
    // Projects

    fun openProject(project: ProjectView) {
        openProject = project
        projectDir = ""
        projectFile = null
    }

    fun closeProject() {
        openProject = null
        projectDir = ""
        projectFile = null
    }

    /** Show a folder of the open project. */
    fun browse(dir: String) {
        projectDir = dir
        projectFile = null
    }

    fun openFile(path: String) {
        projectDir = path.substringBeforeLast('/', "")
        projectFile = path
    }

    /** One step back: file → its folder → the parent folder → the project list → chats. */
    fun projectBack() {
        when {
            projectFile != null -> projectFile = null
            projectDir.isNotEmpty() -> projectDir = projectDir.substringBeforeLast('/', "")
            openProject != null -> closeProject()
            else -> showProjects = false
        }
    }

    /** Start a chat in [project] from its page. */
    fun newChatIn(project: ProjectView) {
        newChatProject = project
        showNewChat = true
    }

    fun takeNewChatProject(): ProjectView? = newChatProject.also { newChatProject = null }

    /** Every indexed project, best matches first. */
    suspend fun findProjects(query: String): Result<List<ProjectView>> = load { findProjects(query, 2000u) }

    suspend fun listDir(project: ProjectView, dir: String): Result<DirView> = load { listDir(project.path, dir) }

    suspend fun readFile(project: ProjectView, path: String): Result<FileView> = load { readFile(project.path, path) }

    suspend fun findFiles(project: ProjectView, query: String): List<FileHitView> =
        load { findFiles(project.path, query) }.getOrDefault(emptyList())

    /** A blocking client call off the main thread; the error reads well on screen. */
    private suspend fun <T> load(block: MothrClient.() -> T): Result<T> {
        val c = client ?: return Result.failure(IllegalStateException("Not connected"))
        return try {
            Result.success(withContext(Dispatchers.IO) { c.block() })
        } catch (e: Throwable) {
            Result.failure(IllegalStateException(e.friendly()))
        }
    }

    fun newChat(
        agentId: String,
        projectPath: String,
        modeId: String,
        modelId: String?,
        effort: String?,
        context: String?,
        prompt: String,
        design: Boolean,
    ) {
        val computer = current ?: return
        if (creatingChat) return
        creatingChat = true
        val c = computer.client
        scope.launch {
            try {
                val id = try {
                    withContext(Dispatchers.IO) { c.newChat(agentId, projectPath, modeId, modelId, effort, context, prompt, design) }
                } finally {
                    creatingChat = false
                }
                prefs.putString(KEY_AGENT, agentId)
                prefs.putString(KEY_MODE, modeId)
                // Default (null) is remembered too, by dropping the key.
                saveChoice(modelKey(agentId), modelId)
                saveChoice(effortKey(agentId), effort)
                saveChoice(contextKey(agentId), context)
                prefs.putString(projectKey(computer.id), projectPath)
                newChatDraft = ""
                if (computer !== current) return@launch
                showNewChat = false
                stack = listOf(id)
                // The chat arrives over the socket a moment later.
                repeat(20) {
                    openChat = c.chat(id)
                    if (openChat != null) return@launch
                    delay(100)
                }
            } catch (e: Throwable) {
                toast = e.friendly()
            }
        }
    }

    private fun saveChoice(key: String, value: String?) {
        if (value != null) prefs.putString(key, value) else prefs.remove(key)
    }

    /** What new chats with [agentId] last started with; null means the agent's default. */
    fun lastModel(agentId: String): String? = prefs.getStringOrNull(modelKey(agentId))
    fun lastEffort(agentId: String): String? = prefs.getStringOrNull(effortKey(agentId))
    fun lastContext(agentId: String): String? = prefs.getStringOrNull(contextKey(agentId))

    val lastAgent: String? get() = prefs.getStringOrNull(KEY_AGENT)
    val lastMode: String? get() = prefs.getStringOrNull(KEY_MODE)

    /** The project last used on this computer; empty means General. */
    val lastProject: String? get() = current?.let { prefs.getStringOrNull(projectKey(it.id)) }

    companion object {
        private const val KEY_LINK = "pairing_link"
        private const val KEY_COMPUTERS = "computers"
        private const val KEY_CURRENT = "current_computer"
        private const val KEY_AGENT = "last_agent"
        private const val KEY_MODE = "last_mode"
        private const val KEY_BACKGROUND = "run_in_background"

        private fun linkKey(id: String) = "computer.$id.link"
        private fun nameKey(id: String) = "computer.$id.name"
        private fun hostKey(id: String) = "computer.$id.host"
        private fun projectKey(id: String) = "computer.$id.project"
        private fun modelKey(agentId: String) = "model.$agentId"
        private fun effortKey(agentId: String) = "effort.$agentId"
        private fun contextKey(agentId: String) = "context.$agentId"
    }
}

/** A message a person can act on. */
fun Throwable.friendly(): String = when (this) {
    is ClientException.InvalidLink -> reason
    is ClientException.Network -> reason
    is ClientException.Rejected -> reason
    is ClientException.Invalid -> reason
    is ClientException.Unauthorized -> "This phone isn't paired anymore. Pair again from Mothr → Settings → Remote."
    else -> message ?: toString()
}
