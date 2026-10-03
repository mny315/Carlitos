package io.github.mny315.carlitos

import android.content.Intent
import android.graphics.Bitmap
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun UiSuite.settingsRecoveryNotice() {
    check(instrumentation.targetContext.packageName.endsWith(".playbacktest"))
    try {
        shell("input keyevent KEYCODE_WAKEUP")
        shell("wm dismiss-keyguard")
        instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        eventually("Settings recovery UI startup") {
            try { call().getDouble("width") > 0 } catch (_: Exception) { false }
        }
        eventually("Settings recovery notice reaches the first Activity") {
            call().getString("notice").contains("Settings recovered:")
        }
        note("damaged settings are reported when the first Activity subscribes")
    } finally {
        shell("input keyevent KEYCODE_SLEEP")
    }
}

@UnstableApi
internal fun UiSuite.feedbackChecks() {
    call("page", "value" to 2)
    call("feedback", "kind" to "scan")
    Thread.sleep(350)
    val spinner = element("Scanning…", "ProgressIndicator")
    val scale = call().getDouble("scale")
    fun spinnerImage(): Bitmap {
        val screen = instrumentation.uiAutomation.takeScreenshot()!!
        return Bitmap.createBitmap(screen, (spinner.getDouble("x") * scale).toInt(), (spinner.getDouble("y") * scale).toInt(),
            (spinner.getDouble("w") * scale).toInt(), (spinner.getDouble("h") * scale).toInt()).also { screen.recycle() }
    }
    val first = spinnerImage()
    Thread.sleep(280)
    val second = spinnerImage()
    check(!first.sameAs(second)) { "Scan indicator must animate" }
    first.recycle(); second.recycle()
    capture("scanning-indicator")
    call("feedback", "kind" to "scan-done")
    call("feedback", "kind" to "imported")
    Thread.sleep(300)
    val notice = element("Notification", "Groupbox")
    val height = notice.getDouble("h")
    check(height <= 60.1) { "Short notice must be compact: $height" }
    check(notice.getDouble("y") < call().getDouble("height") * .2) { "Notice must stay at the top: $notice" }
    val dismiss = element("Dismiss message")
    check(dismiss.getDouble("w") == 48.0 &&
        kotlin.math.abs(dismiss.getDouble("x") + dismiss.getDouble("w") - notice.getDouble("x") - notice.getDouble("w")) < .1) {
        "Dismiss button must leave room for text and stay at the right: $dismiss"
    }
    capture("import-notice")
    eventually("Import notification times out") { call().getString("notice").isEmpty() }
    Thread.sleep(300)
    check(elements().none { it.optString("label") == "Notification" && it.getDouble("h") > .1 })
    call("feedback", "kind" to "updated")
    Thread.sleep(300)
    call("feedback", "kind" to "dismiss")
    Thread.sleep(70)
    val fading = element("Notification", "Groupbox").getDouble("h")
    check(fading > 0 && fading < height) { "Notification must collapse smoothly: $fading" }
    Thread.sleep(300)
    check(elements().none { it.optString("label") == "Notification" && it.getDouble("h") > .1 })
    call("page", "value" to 0)
    note("animated scan indicator, compact notification, timeout and smooth collapse")
}
