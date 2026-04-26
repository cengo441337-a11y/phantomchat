package de.dcinfosec.phantomchat

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity

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
