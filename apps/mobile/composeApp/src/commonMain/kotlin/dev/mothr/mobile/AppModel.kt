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
import dev.mothr.client.MothrClient
import dev.mothr.client.ProjectView
import dev.mothr.client.UsageView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.IO
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The app's state. All data comes from the Rust client (the desktop's own session model and
 * reducer); this class only turns its change notifications into Compose state.
 *
 * One instance per process ([Mothr.model]): while the app is off screen it keeps the connection
 * (if [runInBackground]) and turns requests agents are blocked on into notifications.
 */
class AppModel(private val scope: CoroutineScope) {
    private val prefs = Settings()

    var client: MothrClient? by mutableStateOf(null)
        private set
    var connection: ConnectionInfo? by mutableStateOf(null)
        private set
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

    /** Keep the connection and notifications going while the app is closed. */
    var runInBackground: Boolean by mutableStateOf(prefs.getBoolean(KEY_BACKGROUND, true))
        private set

    private var loop: Job? = null
    private val background get() = Mothr.background

    /** The UI is on screen. Off screen, only the connection and [Attention] are tracked. */
    private var foreground = false

    /** Disconnected from the notification; reconnects when the app is opened. */
    private var paused = false

    /** Requests already seen (on screen or as a notification); each notifies at most once. */
    private val seen = mutableSetOf<String>()

    /** Requests with a notification up. */
    private val shown = mutableSetOf<String>()
    private var lastStatus = ""

    init {
        prefs.getStringOrNull(KEY_LINK)?.let { pair(it) }
    }

    /** Called by the platform when the UI appears or goes away. */
    fun setForeground(on: Boolean) {
        if (foreground == on) return
        foreground = on
        val c = client ?: return
        if (on) {
            if (paused) {
                paused = false
                c.connect()
                background.keepAlive(runInBackground)
            }
            scope.launch { refresh(c) }
        } else if (!runInBackground) {
            scope.launch(Dispatchers.IO) { c.disconnect() }
            paused = true
        }
    }

    fun toggleBackground(on: Boolean) {
        runInBackground = on
        prefs.putBoolean(KEY_BACKGROUND, on)
        if (client != null && !paused) background.keepAlive(on)
    }

    /** "Disconnect" on the ongoing notification: stop until the app is opened again. */
    fun pauseUntilOpened() {
        val c = client ?: return
        paused = true
        background.keepAlive(false)
        scope.launch(Dispatchers.IO) { c.disconnect() }
        clearNotifications()
    }

    /** Open a chat from a notification: [path] is the orchestrator then the subagent. */
    fun openPath(path: List<String>) {
        if (client == null || path.isEmpty()) return
        showNewChat = false
        showUsage = false
        stack = path
        openChat = client?.chat(path.last())
        withdrawChat(path.last())
        act { markRead(path.last()) }
    }

    /**
     * Answer from a notification without opening the app. Returns an error message, or null
     * when the desktop took the answer.
     */
    suspend fun quickReply(chatId: String, entryId: String, kind: AttentionKind, replyId: String): String? {
        val c = client ?: return "This phone isn't paired with Mothr"
        return try {
            withContext(Dispatchers.IO) {
                when (kind) {
                    AttentionKind.PERMISSION -> c.respondPermission(chatId, entryId, replyId)
                    AttentionKind.QUESTION -> c.answerQuestion(chatId, entryId, listOf(listOf(replyId)))
                }
            }
            "$chatId/$entryId".let { if (shown.remove(it)) background.withdraw(it) }
            null
        } catch (e: Throwable) {
            e.friendly().also { background.failed(it) }
        }
    }

    /**
     * One background check (iOS background refresh): connect, wait for the desktop's state,
     * post notifications, and drop the connection again if the app is still off screen.
     */
    suspend fun checkOnce(timeoutMs: Long) {
        val c = client ?: return
        c.connect()
        var rev = 0uL
        val start = kotlin.time.TimeSource.Monotonic.markNow()
        while (start.elapsedNow().inWholeMilliseconds < timeoutMs) {
            val conn = c.connection()
            if (conn.loaded && conn.state == ConnectionState.CONNECTED) break
            if (conn.state == ConnectionState.UNAUTHORIZED) break
            rev = withContext(Dispatchers.IO) { c.waitForChange(rev, 1_000u) }
        }
        refresh(c)
        if (!foreground && !runInBackground) withContext(Dispatchers.IO) { c.disconnect() }
    }

    fun pair(link: String) {
        pairError = null
        try {
            val c = MothrClient(link)
            c.connect()
            prefs.putString(KEY_LINK, c.link())
            client = c
            paused = false
            startLoop(c)
            background.keepAlive(runInBackground)
        } catch (e: Throwable) {
            pairError = e.friendly()
        }
    }

    fun unpair() {
        loop?.cancel()
        background.keepAlive(false)
        clearNotifications()
        seen.clear()
        client?.let { c -> scope.launch(Dispatchers.IO) { c.disconnect(); c.close() } }
        client = null
        connection = null
        groups = emptyList()
        stack = emptyList()
        openChat = null
        prefs.remove(KEY_LINK)
    }

    private fun startLoop(c: MothrClient) {
        loop?.cancel()
        loop = scope.launch {
            var rev = 0uL
            refresh(c)
            while (isActive) {
                rev = withContext(Dispatchers.IO) { c.waitForChange(rev, 15_000u) }
                refresh(c)
                // Streaming produces many tiny changes; ~30 updates/s is plenty.
                delay(33)
            }
        }
    }

    private suspend fun refresh(c: MothrClient) {
        if (!foreground) {
            // Off screen: just the connection and what needs the user.
            val (conn, items) = withContext(Dispatchers.Default) { c.connection() to c.attention() }
            connection = conn
            track(conn, items)
            return
        }
        val id = stack.lastOrNull()
        val q = query
        val snapshot = withContext(Dispatchers.Default) {
            Snapshot(c.connection(), c.chatGroups(q), c.usage(), c.agents(), c.projects(), id?.let { c.chat(it) }, c.takeToasts(), c.attention())
        }
        track(snapshot.connection, snapshot.attention)
        connection = snapshot.connection
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

    /** Notify about new requests (only while off screen) and withdraw answered ones. */
    private fun track(conn: ConnectionInfo, items: List<Attention>) {
        // The list is only meaningful once the desktop's snapshot is in.
        if (conn.loaded) {
            val keys = items.mapTo(HashSet()) { it.key }
            for (a in items) {
                if (seen.add(a.key) && !foreground) {
                    background.show(a)
                    shown += a.key
                }
            }
            seen.retainAll(keys)
            (shown - keys).forEach { shown -= it; background.withdraw(it) }
        }
        val text = when (conn.state) {
            ConnectionState.CONNECTED -> when (val n = conn.needsYou.toInt()) {
                0 -> "Connected to ${conn.server.removePrefix("https://").removePrefix("http://")}"
                1 -> "1 chat needs you"
                else -> "$n chats need you"
            }
            ConnectionState.UNAUTHORIZED -> "Not paired anymore. Open Mothr to pair again."
            ConnectionState.IDLE -> "Disconnected"
            else -> "Reconnecting…"
        }
        if (text != lastStatus) {
            lastStatus = text
            background.status(text)
        }
    }

    private fun withdrawChat(chatId: String) {
        shown.filter { it.startsWith("$chatId/") }.forEach { shown -= it; background.withdraw(it) }
    }

    private fun clearNotifications() {
        shown.forEach { background.withdraw(it) }
        shown.clear()
    }

    fun search(q: String) {
        query = q
        client?.let { c -> scope.launch { groups = withContext(Dispatchers.Default) { c.chatGroups(q) } } }
    }

    fun open(id: String) {
        stack = stack + id
        openChat = client?.chat(id)
        withdrawChat(id)
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

    fun newChat(agentId: String, projectPath: String, modeId: String, prompt: String, design: Boolean) {
        val c = client ?: return
        scope.launch {
            try {
                val id = withContext(Dispatchers.IO) { c.newChat(agentId, projectPath, modeId, null, prompt, design) }
                showNewChat = false
                prefs.putString(KEY_AGENT, agentId)
                prefs.putString(KEY_MODE, modeId)
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

    val lastAgent: String? get() = prefs.getStringOrNull(KEY_AGENT)
    val lastMode: String? get() = prefs.getStringOrNull(KEY_MODE)

    companion object {
        private const val KEY_LINK = "pairing_link"
        private const val KEY_AGENT = "last_agent"
        private const val KEY_MODE = "last_mode"
        private const val KEY_BACKGROUND = "run_in_background"
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
