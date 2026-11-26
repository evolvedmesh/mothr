package dev.mothr.mobile

import android.Manifest
import android.annotation.SuppressLint
import android.app.Application
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import dev.mothr.client.Attention
import dev.mothr.client.AttentionKind

/** Sets up [Mothr.background] before anything touches the model. */
class MothrApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        Mothr.background = AndroidBackground(this)
        Mothr.updates = AndroidUpdates(this)
        // Create the model (and reconnect a paired phone) outside any Compose snapshot.
        Mothr.model
    }
}

/**
 * Android side of [Background]: a foreground service holds the connection, and every request
 * an agent is blocked on becomes a high-priority notification with its quick replies as actions.
 */
class AndroidBackground(private val app: Context) : Background {
    private val nm = NotificationManagerCompat.from(app)

    init {
        nm.createNotificationChannelsCompat(
            listOf(
                androidx.core.app.NotificationChannelCompat.Builder(CHANNEL_NEEDS_YOU, NotificationManager.IMPORTANCE_HIGH)
                    .setName("Needs you")
                    .setDescription("An agent is waiting for a permission or an answer")
                    .build(),
                androidx.core.app.NotificationChannelCompat.Builder(CHANNEL_CONNECTION, NotificationManager.IMPORTANCE_MIN)
                    .setName("Connection")
                    .setDescription("Shown while Mothr stays connected in the background")
                    .setShowBadge(false)
                    .build(),
            ),
        )
    }

    var statusText = "Connecting…"
        private set

    override fun keepAlive(on: Boolean) {
        val intent = Intent(app, ConnectionService::class.java)
        if (on) {
            try {
                ContextCompat.startForegroundService(app, intent)
            } catch (e: IllegalStateException) {
                // Android 12+ refuses to start it from the background (e.g. after a notification
                // action restarted the process). It starts with the app next time.
            }
        } else {
            app.stopService(intent)
        }
    }

    override fun status(text: String) {
        statusText = text
        if (ConnectionService.running) notifyIfAllowed(null, ConnectionService.NOTIFICATION_ID, ConnectionService.build(app, text))
    }

    override fun show(item: Attention, source: NotificationSource) {
        val n = NotificationCompat.Builder(app, CHANNEL_NEEDS_YOU)
            .setSmallIcon(if (item.kind == AttentionKind.PERMISSION) R.drawable.ic_stat_permission else R.drawable.ic_stat_question)
            .setColor(0xFF000000.toInt() or item.agent.color.toInt())
            .setContentTitle(item.title)
            .setContentText(item.body.lineSequence().first())
            .setStyle(NotificationCompat.BigTextStyle().bigText(item.body))
            .setSubText(listOfNotNull(source.label, item.projectName, item.chatTitle).filter { it.isNotBlank() }.distinct().joinToString(" · "))
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setWhen(item.updatedMs)
            .setShowWhen(true)
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setGroup(GROUP)
            .setContentIntent(launchIntent(item.openPath, item.key, source.computerId))
        item.quickReplies.sortedByDescending { it.allow }.forEachIndexed { i, r ->
            val intent = Intent(app, NotificationReceiver::class.java)
                .setAction(NotificationReceiver.ACTION_REPLY)
                .putExtra(NotificationReceiver.EXTRA_KEY, item.key)
                .putExtra(NotificationReceiver.EXTRA_COMPUTER, source.computerId)
                .putExtra(NotificationReceiver.EXTRA_CHAT, item.chatId)
                .putExtra(NotificationReceiver.EXTRA_ENTRY, item.entryId)
                .putExtra(NotificationReceiver.EXTRA_KIND, item.kind.name)
                .putExtra(NotificationReceiver.EXTRA_REPLY, r.id)
            val pi = PendingIntent.getBroadcast(app, (item.key + i).hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            n.addAction(0, r.label, pi)
        }
        notifyIfAllowed(item.key, ATTENTION_ID, n.build())
    }

    override fun withdraw(key: String) {
        nm.cancel(key, ATTENTION_ID)
    }

    override fun failed(text: String) {
        val n = NotificationCompat.Builder(app, CHANNEL_NEEDS_YOU)
            .setSmallIcon(R.drawable.ic_stat_connected)
            .setContentTitle("Couldn't send your answer")
            .setContentText(text)
            .setAutoCancel(true)
            .setContentIntent(launchIntent(emptyList()))
            .build()
        notifyIfAllowed("failed", ATTENTION_ID, n)
    }

    private fun launchIntent(path: List<String>, key: String = "", computerId: String? = null) = PendingIntent.getActivity(
        app,
        key.hashCode(),
        Intent(app, MainActivity::class.java)
            .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(MainActivity.EXTRA_COMPUTER, computerId)
            .putExtra(MainActivity.EXTRA_OPEN_PATH, path.toTypedArray()),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    @SuppressLint("MissingPermission") // checked by canNotify()
    private fun notifyIfAllowed(tag: String?, id: Int, n: android.app.Notification) {
        if (canNotify(app)) nm.notify(tag, id, n)
    }

    companion object {
        const val CHANNEL_NEEDS_YOU = "needs_you"
        const val CHANNEL_CONNECTION = "connection"
        private const val GROUP = "dev.mothr.mobile.NEEDS_YOU"
        private const val ATTENTION_ID = 2

        fun canNotify(context: Context) = Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
    }
}
