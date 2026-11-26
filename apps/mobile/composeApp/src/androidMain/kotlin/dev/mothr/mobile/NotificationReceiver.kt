package dev.mothr.mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.mothr.client.AttentionKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** Allow / Deny / an answer tapped on a "Needs you" notification. */
class NotificationReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_REPLY) return
        val computer = intent.getStringExtra(EXTRA_COMPUTER) ?: return
        val chat = intent.getStringExtra(EXTRA_CHAT) ?: return
        val entry = intent.getStringExtra(EXTRA_ENTRY) ?: return
        val reply = intent.getStringExtra(EXTRA_REPLY) ?: return
        val kind = intent.getStringExtra(EXTRA_KIND)?.let { k -> AttentionKind.entries.find { it.name == k } } ?: return
        // Take it off the shade right away; a failure posts its own notification.
        intent.getStringExtra(EXTRA_KEY)?.let { Mothr.background.withdraw(it) }
        val pending = goAsync()
        CoroutineScope(Dispatchers.Main).launch {
            try {
                withTimeoutOrNull(9_000) { Mothr.model.quickReply(computer, chat, entry, kind, reply) }
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_REPLY = "dev.mothr.mobile.REPLY"
        const val EXTRA_KEY = "key"
        const val EXTRA_COMPUTER = "computer"
        const val EXTRA_CHAT = "chat"
        const val EXTRA_ENTRY = "entry"
        const val EXTRA_KIND = "kind"
        const val EXTRA_REPLY = "reply"
    }
}
