package com.example.mobile

import android.os.Bundle
import android.os.Environment
import android.util.Log
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import io.flutter.embedding.android.FlutterActivity
import java.io.File
import java.io.FileOutputStream

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        logToFile("PHANTOMCHAT_BOOT: MainActivity Start")
        try {
            super.onCreate(savedInstanceState)
            logToFile("PHANTOMCHAT_BOOT: super.onCreate success")
        } catch (e: Exception) {
            logToFile("PHANTOMCHAT_BOOT_ERROR: ${e.message}")
            Log.e("PhantomChat", "Native boot failure", e)
        }
    }

    private fun logToFile(message: String) {
        try {
            val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            val file = File(dir, "phantomchat_debug.log")
            FileOutputStream(file, true).use { 
                it.write("${System.currentTimeMillis()}: $message\n".toByteArray()) 
            }
        } catch (e: Exception) {
            Log.e("PhantomChat", "FS Logging Failed", e)
        }
    }
}
