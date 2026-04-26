package de.dcinfosec.phantomchat

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * PhantomChat — Wave 8B
 *
 * Re-launches [RelayForegroundService] after a device reboot, but ONLY
 * when the user has explicitly opted in via the Settings screen
 * ("Bei Geräte-Start automatisch starten"). The opt-in flag is mirrored
 * into the default `SharedPreferences` bucket on the Dart side so we can
 * read it from this native receiver without spinning up a Flutter
 * isolate just to make a decision.
 *
 * Defaults: **OFF** — privacy-by-default. A messenger that silently
 * re-attaches itself to the network at every boot is a metadata leak
 * waiting to happen, so we require explicit consent.
 */
class BootReceiver : BroadcastReceiver() {

    companion object {
        private const val TAG = "PhantomBoot"

        /**
         * Mirror of the Dart-side opt-in toggle. Stored in the default
         * `SharedPreferences` (`<package>_preferences`) by
         * `shared_preferences` with the `flutter.` prefix it always
         * applies under the hood.
         */
        private const val PREFS_NAME = "de.dcinfosec.phantomchat_preferences"
        private const val KEY_AUTOSTART = "flutter.phantom_bg_autostart_on_boot"
        private const val KEY_APP_LOCK_ENABLED = "flutter.phantom_app_lock_enabled"
    }

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val autoStart = prefs.getBoolean(KEY_AUTOSTART, false)
        // App-lock-enabled flag also flips on the bg service so a locked
        // device can still receive (and queue notifications for) messages
        // even before the user has unlocked the UI.
        val appLockOn = prefs.getBoolean(KEY_APP_LOCK_ENABLED, false)

        if (!autoStart && !appLockOn) {
            Log.i(TAG, "BOOT_COMPLETED: auto-start disabled, NOT launching foreground service")
            return
        }

        Log.i(TAG, "BOOT_COMPLETED: opt-in detected (autoStart=$autoStart, appLock=$appLockOn) — starting service")
        RelayForegroundService.start(context)
    }
}
