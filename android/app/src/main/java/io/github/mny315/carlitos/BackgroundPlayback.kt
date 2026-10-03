package io.github.mny315.carlitos

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings
import android.util.Log

/** Battery exemptions belong to Android; only remember whether we already asked. */
internal object BackgroundPlayback {
    private const val PREFERENCES = "background-playback"
    private const val REQUESTED = "battery-optimization-requested"

    fun requestOnce(activity: Activity) {
        val preferences = activity.getSharedPreferences(PREFERENCES, Activity.MODE_PRIVATE)
        if (preferences.getBoolean(REQUESTED, false)) return
        // Save before leaving the Activity, including when the user declines.
        preferences.edit().putBoolean(REQUESTED, true).apply()
        if (!isUnrestricted(activity)) openSettings(activity)
    }

    fun openSettings(activity: Activity): Boolean {
        val packageUri = Uri.fromParts("package", activity.packageName, null)
        if (!isUnrestricted(activity) && launch(activity,
                Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, packageUri))) return true
        // Some device vendors omit the direct request dialog.
        return launch(activity, Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) ||
            launch(activity, Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, packageUri))
    }

    private fun isUnrestricted(activity: Activity): Boolean =
        activity.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(activity.packageName)

    private fun launch(activity: Activity, intent: Intent): Boolean {
        return try {
            activity.startActivity(intent)
            true
        } catch (error: ActivityNotFoundException) {
            Log.w("Carlitos", "Battery settings unavailable: ${intent.action}", error)
            false
        } catch (error: SecurityException) {
            Log.w("Carlitos", "Battery settings inaccessible: ${intent.action}", error)
            false
        }
    }
}
