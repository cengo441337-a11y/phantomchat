package de.dcinfosec.phantomchat

import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity

/**
 * PhantomChat host activity.
 *
 * Responsibilities beyond the default [FlutterActivity]:
 *
 * 1. **FLAG_SECURE** — blocks screenshots, screen-recording, and the
 *    OS-level app-switcher preview. Critical for the "no message content
 *    leaves the app" guarantee that backs the §203 StGB pitch (medical /
 *    legal-confidentiality use-cases): even another app cannot capture the
 *    chat surface, and the recents-screen thumbnail is rendered as a solid
 *    colour by the system.
 *
 * 2. **Defensive boot** — wraps `super.onCreate` so that a hard crash in
 *    the embedding/native bridge never leaves the user staring at a system
 *    "App keeps stopping" dialog without a logcat trace.
 */
class MainActivity : FlutterActivity() {
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
    }
}
