package de.dcinfosec.phantomchat

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * PhantomChat host activity — combines:
 *   1. FLAG_SECURE (anti-screenshot/recording — §203 StGB pitch)
 *   2. Notification channels (Android 8+, locale-aware DE/EN)
 *   3. Defensive boot (logcat trace on hard native crash)
 */
class MainActivity : FlutterActivity() {

    companion object {
        const val CHANNEL_BACKGROUND = "phantomchat_background"
        const val CHANNEL_MESSAGES = "phantomchat_messages"
        const val CHANNEL_GROUPS = "phantomchat_groups"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Apply BEFORE super.onCreate so the very first frame the OS captures
        // for the recents-screen thumbnail is already secured.
        window.setFlags(
            WindowManager.LayoutParams.FLAG_SECURE,
            WindowManager.LayoutParams.FLAG_SECURE,
        )

        try {
            super.onCreate(savedInstanceState)
        } catch (e: Exception) {
            Log.e("PhantomChat", "Native boot failure", e)
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            createNotificationChannels()
        }
    }

    // Bridge for the in-app APK updater: lets Dart check + request the
    // "install unknown apps" permission (Android 8+ blocks installs without
    // it, which made the in-app update silently do nothing).
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "argos/installer",
        ).setMethodCallHandler { call, result ->
            when (call.method) {
                "canInstall" -> {
                    val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        packageManager.canRequestPackageInstalls()
                    } else {
                        true
                    }
                    result.success(ok)
                }
                "openInstallSettings" -> {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        val intent = Intent(
                            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                            Uri.parse("package:$packageName"),
                        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        startActivity(intent)
                    }
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun createNotificationChannels() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val isDe = resources.configuration.locales[0].language == "de"

        val bg = NotificationChannel(
            CHANNEL_BACKGROUND,
            if (isDe) "Hintergrund-Empfang" else "Background reception",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = if (isDe)
                "Persistente Benachrichtigung während PhantomChat im Hintergrund auf neue Nachrichten lauscht."
            else
                "Persistent notification while PhantomChat listens for new messages in the background."
            setShowBadge(false); enableVibration(false); setSound(null, null)
        }
        val msg = NotificationChannel(
            CHANNEL_MESSAGES,
            if (isDe) "Nachrichten" else "Messages",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = if (isDe) "Eingehende verschlüsselte 1:1-Nachrichten."
                          else "Incoming end-to-end encrypted direct messages."
            enableVibration(true)
        }
        val grp = NotificationChannel(
            CHANNEL_GROUPS,
            if (isDe) "Gruppen" else "Groups",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = if (isDe) "Eingehende Gruppen-Nachrichten (MLS)."
                          else "Incoming group messages (MLS)."
            enableVibration(true)
        }

        nm.createNotificationChannel(bg)
        nm.createNotificationChannel(msg)
        nm.createNotificationChannel(grp)
    }
}
