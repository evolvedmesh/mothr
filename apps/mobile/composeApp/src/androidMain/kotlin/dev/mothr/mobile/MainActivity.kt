package dev.mothr.mobile

import android.Manifest
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        applyBars()
        super.onCreate(savedInstanceState)
        setContent { App() }
        if (savedInstanceState == null) handle(intent)
        // Notifications are the point of staying connected; ask once the phone is paired.
        lifecycleScope.launch {
            snapshotFlow { Mothr.model.client != null }.filter { it }.first()
            if (Build.VERSION.SDK_INT >= 33 && !AndroidBackground.canNotify(this@MainActivity)) {
                askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onStart() {
        super.onStart()
        Mothr.model.setForeground(true)
    }

    override fun onStop() {
        Mothr.model.setForeground(false)
        super.onStop()
    }

    /** A tapped "Needs you" notification opens its chat. */
    private fun handle(intent: Intent?) {
        val path = intent?.getStringArrayExtra(EXTRA_OPEN_PATH)?.toList().orEmpty()
        if (path.isNotEmpty()) Mothr.model.openPath(path)
    }

    // The activity handles uiMode itself (so an open chat survives a theme switch); the system
    // bar icons still have to follow light/dark.
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applyBars()
    }

    private fun applyBars() {
        val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
        val style = if (night) SystemBarStyle.dark(Color.TRANSPARENT) else SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
        enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
    }

    companion object {
        const val EXTRA_OPEN_PATH = "open_path"
    }
}
