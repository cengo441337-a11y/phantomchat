package de.dcinfosec.phantomchat

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat

/**
 * PhantomChat — Wave 8B
 *
 * Long-running foreground service that hosts the relay-listener loop so
 * that incoming messages are decoded and persisted even when the user has
 * swiped the app away from the recents screen.
 *
 * The service is **opt-in**: it must be started explicitly from the
 * Settings UI (`settings_background.dart`) and is never started by the
 * activity lifecycle alone — privacy-by-default.
 *
 * The actual networking work is performed inside a Flutter background
 * isolate spun up by the `flutter_background_service` plugin (see
 * `mobile/lib/services/background_service_config.dart`). This Kotlin
 * service is responsible for:
 *
 *   1. Posting the persistent foreground notification (mandatory for
 *      `android:foregroundServiceType="dataSync"` services on API 34+).
 *   2. Acquiring a partial wake-lock so the CPU does not sleep while we
 *      are processing relay traffic.
 *   3. Handling the notification's "Stop" action so the user can take the
 *      service down without opening the app.
 */
class RelayForegroundService : Service() {

    companion object {
        private const val TAG = "PhantomBgSvc"

        /** Notification ID — unique per process; arbitrary positive int. */
        const val NOTIFICATION_ID = 0x9001

        /** Action sent by the notification's "Stop" button. */
        const val ACTION_STOP_SERVICE = "de.dcinfosec.phantomchat.STOP_RELAY_SERVICE"

        /**
         * 24h auto-release wake-lock.
         *
         * Rationale: Android will silently kill held wake-locks during a
         * Doze maintenance window if they exceed common manufacturer
         * thresholds. 24h is long enough to outlast a normal usage day
         * (so we don't drop messages mid-afternoon) but short enough that
         * a stuck service can't drain the battery indefinitely. The
         * background isolate re-acquires the lock when it next runs.
         */
        private const val WAKE_LOCK_TIMEOUT_MS: Long = 24L * 60L * 60L * 1000L

        private const val WAKE_LOCK_TAG = "PhantomChat:RelayForegroundService"

        /**
         * Convenience: start the service from anywhere (e.g. BootReceiver
         * or a Flutter MethodChannel call).
         */
        fun start(context: Context) {
            val intent = Intent(context, RelayForegroundService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            val intent = Intent(context, RelayForegroundService::class.java).apply {
                action = ACTION_STOP_SERVICE
            }
            context.startService(intent)
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_SERVICE) {
            Log.i(TAG, "Stop action received, tearing down foreground service")
            releaseWakeLock()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }

        startInForeground()
        acquireWakeLock()
        // The Flutter background-isolate (started separately by
        // `flutter_background_service`) hosts the actual relay loop. We
        // intentionally do not duplicate that logic here in Kotlin — it
        // would diverge from the in-app listener and break the prefix-
        // dispatch invariants set up in PR #4.
        Log.i(TAG, "RelayForegroundService active (wake-lock held, notification posted)")

        // START_STICKY: if the OS kills us we want a restart attempt.
        return START_STICKY
    }

    override fun onDestroy() {
        releaseWakeLock()
        super.onDestroy()
    }

    private fun startInForeground() {
        val notification = buildNotification(relayCount = 0)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(relayCount: Int): Notification {
        val openAppIntent = packageManager
            .getLaunchIntentForPackage(packageName)
            ?.apply { flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP }

        val contentPi: PendingIntent? = if (openAppIntent != null) {
            PendingIntent.getActivity(
                this,
                0,
                openAppIntent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        } else null

        val stopIntent = Intent(this, RelayForegroundService::class.java).apply {
            action = ACTION_STOP_SERVICE
        }
        val stopPi = PendingIntent.getService(
            this,
            1,
            stopIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val title = "PhantomChat"
        val body = if (relayCount > 0) {
            "Verbunden mit $relayCount Relays"
        } else {
            "Hintergrund-Empfang aktiv"
        }

        return NotificationCompat.Builder(this, MainActivity.CHANNEL_BACKGROUND)
            .setContentTitle(title)
            .setContentText(body)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)               // sticky — required for FGS
            .setShowWhen(false)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setContentIntent(contentPi)
            .addAction(
                android.R.drawable.ic_menu_close_clear_cancel,
                "Stop",
                stopPi,
            )
            .build()
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG).apply {
            setReferenceCounted(false)
            acquire(WAKE_LOCK_TIMEOUT_MS)
        }
    }

    private fun releaseWakeLock() {
        try {
            wakeLock?.takeIf { it.isHeld }?.release()
        } catch (e: RuntimeException) {
            Log.w(TAG, "wake-lock release failed", e)
        } finally {
            wakeLock = null
        }
    }
}
