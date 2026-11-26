package dev.mothr.mobile

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.IntentCompat
import dev.mothr.client.checkAppUpdate
import dev.mothr.client.downloadAppUpdate
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Updates from Mothr's GitHub releases. The shared Rust client finds the newest release and
 * downloads its APK for this device's ABI, checked against the release's SHA256SUMS; Android's
 * PackageInstaller installs it. The first update asks to allow installs from Mothr and to
 * confirm; on Android 12+ later ones can go through without asking, as Mothr then installed
 * itself.
 *
 * Debug builds can test against another releases API:
 * `adb shell run-as dev.mothr.mobile sh -c 'echo http://10.0.2.2:8799/api > files/update-api'`.
 */
class AndroidUpdates(private val app: Context) : AppUpdates {
    override val currentVersion: String =
        app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: "0.0.0"

    private val abi = Build.SUPPORTED_ABIS.firstOrNull() ?: "arm64-v8a"
    private val dir get() = File(app.cacheDir, "updates")
    private fun apk(offer: UpdateOffer) = File(dir, "mothr-${offer.version}.apk")

    init {
        // Downloads that aren't newer than what's installed (e.g. the one just installed).
        dir.listFiles()?.forEach { f ->
            val v = f.name.removePrefix("mothr-").removeSuffix(".apk")
            if (!isNewer(v, currentVersion)) f.delete()
        }
    }

    private fun api(): String? {
        if (app.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return null
        return File(app.filesDir, "update-api").takeIf { it.isFile }?.readText()?.trim()?.ifEmpty { null }
    }

    override suspend fun check(): UpdateOffer? = withContext(Dispatchers.IO) {
        checkAppUpdate(currentVersion, abi, api())?.let { UpdateOffer(it.version, it.url, it.size.toLong()) }
    }

    override fun isDownloaded(offer: UpdateOffer) = apk(offer).isFile

    override suspend fun download(offer: UpdateOffer) = withContext(Dispatchers.IO) {
        dir.mkdirs()
        // Only the newest download is worth keeping.
        dir.listFiles()?.filter { it != apk(offer) }?.forEach { it.delete() }
        if (!apk(offer).isFile) downloadAppUpdate(offer.version, abi, apk(offer).path, api())
    }

    override fun install(offer: UpdateOffer) {
        val file = apk(offer)
        if (!file.isFile) return
        // Sideloaded apps need the user's OK to install packages, once.
        if (!app.packageManager.canRequestPackageInstalls()) {
            app.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${app.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            return
        }
        val installer = app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(app.packageName)
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            file.inputStream().use { input ->
                session.openWrite("mothr.apk", 0, file.length()).use { out ->
                    input.copyTo(out)
                    session.fsync(out)
                }
            }
            val status = Intent(app, UpdateReceiver::class.java).setAction(UpdateReceiver.ACTION)
            // The installer fills in the result, so this one must be mutable.
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
            session.commit(PendingIntent.getBroadcast(app, id, status, flags).intentSender)
        }
    }
}

/** Whether version [a] is newer than [b] (`1.2.3`, prerelease suffixes ignored). */
internal fun isNewer(a: String, b: String): Boolean {
    fun parts(v: String) = v.substringBefore('-').split('.').map { it.toIntOrNull() ?: 0 }
    val (x, y) = parts(a) to parts(b)
    for (i in 0 until maxOf(x.size, y.size)) {
        val d = x.getOrElse(i) { 0 } - y.getOrElse(i) { 0 }
        if (d != 0) return d > 0
    }
    return false
}

/**
 * Android doesn't restart an app it just updated. This brings the process back up after an
 * update, so [MothrApplication] reconnects to the paired computers and the background
 * connection (and its notifications) carry on.
 */
class UpdatedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {}
}

/** Results from the system installer: ask the user to confirm, or report a failure. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirm = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_INTENT, Intent::class.java) ?: return
                context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            // Android replaces and restarts the app.
            PackageInstaller.STATUS_SUCCESS -> {}
            PackageInstaller.STATUS_FAILURE_ABORTED -> Mothr.model.updateFailed(null, cancelled = true)
            else -> Mothr.model.updateFailed(intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE), cancelled = false)
        }
    }

    companion object {
        const val ACTION = "dev.mothr.mobile.UPDATE_STATUS"
    }
}
