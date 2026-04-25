package de.dcinfosec.phantomchat

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity

class MainActivity : FlutterActivity() {

    companion object {
        /** Foreground-service channel — low importance, persistent, no sound. */
        const val CHANNEL_BACKGROUND = "phantomchat_background"

        /** Incoming 1:1 message channel — high importance, default sound. */
        const val CHANNEL_MESSAGES = "phantomchat_messages"

        /** Group / MLS message channel — high importance, default sound. */
        const val CHANNEL_GROUPS = "phantomchat_groups"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Channels MUST exist before any notification is posted on Android 8+.
        // Cheap and idempotent — safe to call on every activity launch.
        createNotificationChannels()
    }

    /**
     * Create the three notification channels PhantomChat uses.
     *
     * Strings are written in DE+EN (the user base is German-primary but
     * we keep an English follow-on so the notification surface remains
     * legible if the user switches their device locale).
     */
    private fun createNotificationChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return

        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

        val locale = resources.configuration.locales[0].language
        val isDe = locale.equals("de", ignoreCase = true)

        // 1. Background relay-listener — IMPORTANCE_LOW = no sound, no peek.
        val bg = NotificationChannel(
            CHANNEL_BACKGROUND,
            if (isDe) "Hintergrund-Verbindung" else "Background connection",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = if (isDe) {
                "Persistente Benachrichtigung wenn PhantomChat im Hintergrund auf neue Nachrichten wartet."
            } else {
                "Persistent notification while PhantomChat listens for new messages in the background."
            }
            setShowBadge(false)
            enableVibration(false)
            setSound(null, null)
        }

        // 2. 1:1 messages — IMPORTANCE_HIGH = head-up + sound.
        val msg = NotificationChannel(
            CHANNEL_MESSAGES,
            if (isDe) "Nachrichten" else "Messages",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = if (isDe) {
                "Eingehende verschlüsselte 1:1-Nachrichten."
            } else {
                "Incoming end-to-end encrypted direct messages."
            }
            enableVibration(true)
        }

        // 3. Group / MLS messages — IMPORTANCE_HIGH = head-up + sound.
        val grp = NotificationChannel(
            CHANNEL_GROUPS,
            if (isDe) "Gruppen" else "Groups",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = if (isDe) {
                "Eingehende Gruppen-Nachrichten (MLS)."
            } else {
                "Incoming group messages (MLS)."
            }
            enableVibration(true)
        }

        nm.createNotificationChannel(bg)
        nm.createNotificationChannel(msg)
        nm.createNotificationChannel(grp)
    }
}
