package io.github.mny315.carlitos

import android.Manifest
import android.app.Activity
import android.app.Instrumentation
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.os.ParcelFileDescriptor
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings

/** Exercises lifecycle and request routing without accepting a real user dialog. */
internal class BatteryOptimizationSuite(private val instrumentation: Instrumentation) {
    private val context get() = instrumentation.targetContext
    private fun main(action: () -> Unit) {
        var result: Result<Unit>? = null
        instrumentation.runOnMainSync { result = runCatching(action) }
        checkNotNull(result).getOrThrow()
    }
    private fun shell(command: String) = instrumentation.uiAutomation.executeShellCommand(command).use {
        ParcelFileDescriptor.AutoCloseInputStream(it).use { input -> input.readBytes() }
    }
    private fun eventually(message: String, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 10000
        while (SystemClock.elapsedRealtime() < deadline) {
            if (condition()) return
            Thread.sleep(50)
        }
        error(message)
    }
    fun run() {
        val packageName = context.packageName
        check(packageName.endsWith(".playbacktest")) { "Use the isolated playback/UI test app" }
        check(context.checkSelfPermission(Manifest.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS) ==
            PackageManager.PERMISSION_GRANTED) { "Battery exemption permission is missing" }
        val power = context.getSystemService(PowerManager::class.java)
        val wasExempt = power.isIgnoringBatteryOptimizations(packageName)
        val preferences = context.getSharedPreferences("background-playback", Activity.MODE_PRIVATE)
        val requested = "battery-optimization-requested"
        val wasRequested = preferences.getBoolean(requested, false)
        val requests = mutableListOf<Intent>()
        val unavailable = mutableSetOf<String>()
        val actions = listOf(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
            Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS, Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action !in actions) return null
                requests.add(Intent(intent))
                if (intent.action in unavailable) throw ActivityNotFoundException(intent.action)
                return Instrumentation.ActivityResult(Activity.RESULT_CANCELED, null)
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            shell("input keyevent KEYCODE_WAKEUP")
            shell("wm dismiss-keyguard")
            shell("dumpsys deviceidle whitelist -$packageName")
            eventually("Could not remove the test app's battery exemption") { !power.isIgnoringBatteryOptimizations(packageName) }
            check(preferences.edit().remove(requested).commit())
            instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            instrumentation.waitForIdleSync()
            var previous: MainActivity? = null
            main {
                check(requests.size == 1 && requests.single().action == actions[0]) { "First launch did not request an exemption: $requests" }
                check(requests.single().data.toString() == "package:$packageName")
                check(preferences.getBoolean(requested, false))
                previous = checkNotNull(Bridge.activity.get())
                previous!!.recreate()
            }
            eventually("Activity did not resume after recreation") {
                var ready = false
                main { ready = Bridge.activity.get()?.let { it !== previous && it.hasWindowFocus() } == true }
                ready
            }
            main { check(requests.size == 1) { "Refusal was forgotten after Activity recreation" } }
            Bridge.dispatch("{\"op\":\"battery-settings\"}")
            instrumentation.waitForIdleSync()
            main {
                check(requests.size == 2 && requests.last().action == actions[0]) { "Manual retry did not reopen the request" }
                requests.clear()
                unavailable.add(actions[0])
                check(BackgroundPlayback.openSettings(checkNotNull(Bridge.activity.get())))
                check(requests.map { it.action } == actions.take(2)) { "Missing dialog did not fall back to battery settings" }
                requests.clear()
                unavailable.add(actions[1])
                check(BackgroundPlayback.openSettings(checkNotNull(Bridge.activity.get())))
                check(requests.map { it.action } == actions) { "Missing battery settings did not fall back to app details" }
                requests.clear()
                unavailable.add(actions[2])
                check(!BackgroundPlayback.openSettings(checkNotNull(Bridge.activity.get())))
                unavailable.clear()
                requests.clear()
            }
            shell("dumpsys deviceidle whitelist +$packageName")
            eventually("Could not exempt the test app") { power.isIgnoringBatteryOptimizations(packageName) }
            check(preferences.edit().remove(requested).commit())
            main {
                val activity = checkNotNull(Bridge.activity.get())
                BackgroundPlayback.requestOnce(activity)
                check(requests.isEmpty()) { "Already exempt app prompted again" }
                check(BackgroundPlayback.openSettings(activity))
                check(requests.single().action == actions[1]) { "Exempt app did not open battery settings" }
                requests.clear()
            }
            shell("dumpsys deviceidle whitelist -$packageName")
            eventually("Could not revoke the test app's exemption") { !power.isIgnoringBatteryOptimizations(packageName) }
            main {
                val activity = checkNotNull(Bridge.activity.get())
                BackgroundPlayback.requestOnce(activity)
                check(requests.isEmpty()) { "Revocation triggered another automatic prompt" }
                check(BackgroundPlayback.openSettings(activity))
                check(requests.single().action == actions[0]) { "Manual retry used stale exemption state" }
            }
        } finally {
            // Finish before restoring onboarding state, so a final resume cannot
            // leave a real system dialog behind when instrumentation exits.
            main { Bridge.activity.get()?.finish() }
            instrumentation.waitForIdleSync()
            instrumentation.removeMonitor(monitor)
            if (wasRequested) preferences.edit().putBoolean(requested, true).commit()
            else preferences.edit().remove(requested).commit()
            shell("dumpsys deviceidle whitelist ${if (wasExempt) "+" else "-"}$packageName")
            shell("input keyevent KEYCODE_SLEEP")
        }
    }
}
