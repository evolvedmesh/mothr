package dev.mothr.mobile

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps the process, and with it the connection to the desktop, alive while the app is closed.
 * The connection itself lives in [Mothr.model]; this service only holds the foreground slot and
 * shows a quiet ongoing notification ("Connected to …" / "2 chats need you").
 */
class ConnectionService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_DISCONNECT) {
            Mothr.model.pauseUntilOpened()
            return START_NOT_STICKY
        }
        val text = (Mothr.background as? AndroidBackground)?.statusText ?: "Connecting…"
        val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, build(this, text), type)
        running = true
        // Restarted by the system after the process was killed: bring the connection back.
        Mothr.model
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    companion object {
        const val NOTIFICATION_ID = 1
        private const val ACTION_DISCONNECT = "dev.mothr.mobile.DISCONNECT"

        @Volatile
        var running = false
            private set

        fun build(context: Context, text: String): Notification {
            val open = PendingIntent.getActivity(
                context, 0,
                Intent(context, MainActivity::class.java).setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val disconnect = PendingIntent.getService(
                context, 1,
                Intent(context, ConnectionService::class.java).setAction(ACTION_DISCONNECT),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            return NotificationCompat.Builder(context, AndroidBackground.CHANNEL_CONNECTION)
                .setSmallIcon(R.drawable.ic_stat_connected)
                .setContentTitle(text)
                .setOngoing(true)
                .setSilent(true)
                .setShowWhen(false)
                .setPriority(NotificationCompat.PRIORITY_MIN)
                .setCategory(NotificationCompat.CATEGORY_SERVICE)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
                .setContentIntent(open)
                .addAction(0, "Disconnect", disconnect)
                .build()
        }
    }
}
