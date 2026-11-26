package dev.mothr.mobile

import dev.mothr.client.Attention
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** Which computer a notification is about. [label] is set when several are paired. */
data class NotificationSource(val computerId: String, val label: String?)

/**
 * What a platform does while the app is off screen. Android keeps the connection in a
 * foreground service and posts notifications; iOS posts local notifications and checks in
 * through background refresh (see iosApp/iosApp/Background.swift).
 */
interface Background {
    /** Paired and allowed to run in the background: keep the connections alive. */
    fun keepAlive(on: Boolean)

    /** One line for the ongoing "connected" notification (Android). */
    fun status(text: String)

    /** An agent is waiting on the user. Called once per request, when it isn't on screen. */
    fun show(item: Attention, source: NotificationSource)

    /** The request was answered (here, on the desktop or elsewhere) or the chat is gone. */
    fun withdraw(key: String)

    /** A notification action failed. */
    fun failed(text: String)
}

private object NoBackground : Background {
    override fun keepAlive(on: Boolean) {}
    override fun status(text: String) {}
    override fun show(item: Attention, source: NotificationSource) {}
    override fun withdraw(key: String) {}
    override fun failed(text: String) {}
}

/** A newer release with a build for this device. */
data class UpdateOffer(val version: String, val url: String, val size: Long)

/**
 * Updating the app outside a store. Android installs APKs from Mothr's GitHub releases; on iOS
 * the App Store does it, so there is no [Mothr.updates] there.
 */
interface AppUpdates {
    /** This app's version, e.g. `0.1.0`. */
    val currentVersion: String

    /** The newest release if it's newer than this app and has a build for this device. */
    suspend fun check(): UpdateOffer?

    /** Whether [offer] is already downloaded and verified. */
    fun isDownloaded(offer: UpdateOffer): Boolean

    /** Download [offer] and check it against the release's checksums. */
    suspend fun download(offer: UpdateOffer)

    /** Hand the downloaded update to the system installer (it may ask the user to confirm). */
    fun install(offer: UpdateOffer)
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

    /** Set by platforms that update the app themselves (Android). */
    var updates: AppUpdates? = null

    val model: AppModel by lazy { AppModel(CoroutineScope(SupervisorJob() + Dispatchers.Main)) }
}
