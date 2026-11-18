package dev.mothr.mobile

import dev.mothr.client.Attention
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/**
 * What a platform does while the app is off screen. Android keeps the connection in a
 * foreground service and posts notifications; iOS posts local notifications and checks in
 * through background refresh (see iosApp/iosApp/Background.swift).
 */
interface Background {
    /** Paired and allowed to run in the background: keep the connection alive. */
    fun keepAlive(on: Boolean)

    /** One line for the ongoing "connected" notification (Android). */
    fun status(text: String)

    /** An agent is waiting on the user. Called once per request, only while off screen. */
    fun show(item: Attention)

    /** The request was answered (here, on the desktop or elsewhere) or the chat is gone. */
    fun withdraw(key: String)

    /** A notification action failed. */
    fun failed(text: String)
}

private object NoBackground : Background {
    override fun keepAlive(on: Boolean) {}
    override fun status(text: String) {}
    override fun show(item: Attention) {}
    override fun withdraw(key: String) {}
    override fun failed(text: String) {}
}

/**
 * The app's single model. It lives as long as the process, not the screen, so the connection
 * and the notifications keep going after the UI is gone.
 */
object Mothr {
    /**
     * Set by the platform before [model] is first used. The platform also creates [model] at
     * startup: its constructor writes Compose state, which must not happen inside a snapshot.
     */
    var background: Background = NoBackground

    val model: AppModel by lazy { AppModel(CoroutineScope(SupervisorJob() + Dispatchers.Main)) }
}
