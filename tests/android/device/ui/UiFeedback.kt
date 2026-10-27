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
    for ((theme, name) in listOf(0 to "dark", 1 to "light")) {
        call("appearance", "theme" to theme)
        call("feedback", "kind" to "imported")
        Thread.sleep(300)
        val notice = element("Notification", "Groupbox")
        val height = notice.getDouble("h")
        check(height <= 72.1) { "Short notice including its gaps must be compact: $height" }
        check(notice.getDouble("y") < call().getDouble("height") * .2) { "Notice must stay at the top: $notice" }
        val dismiss = element("Dismiss message")
        check(dismiss.getDouble("y") - notice.getDouble("y") >= 11.9) {
            "Notification card needs a 12 dp gap below the header: $dismiss / $notice"
        }
        check(dismiss.getDouble("w") == 48.0 &&
            kotlin.math.abs(dismiss.getDouble("x") + dismiss.getDouble("w") - notice.getDouble("x") - notice.getDouble("w")) < .1) {
            "Dismiss button must leave room for text and stay at the right: $dismiss"
        }
        val progress = element("Notification time remaining", "ProgressIndicator")
        check(progress.getDouble("h") == 3.0 && progress.getDouble("w") > 100)
        val initial = call().getDouble("notice_remaining")
        check(initial > .8 && initial < 1)
        capture("import-notice-$name")
        Thread.sleep(1100)
        check(call().getDouble("notice_remaining") < initial - .2) { "Countdown must decrease" }
        capture("import-notice-$name-countdown")
        // A repeated completion must restart even when the text is unchanged.
        check(call("feedback", "kind" to "imported").getDouble("notice_remaining") > .95)
        eventually("Import notification times out") { call().getString("notice").isEmpty() }
        check(!call().getBoolean("notice_timed"))
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
    }
    call("feedback", "kind" to "updated")
    call("feedback", "kind" to "error")
    check(!call().getBoolean("notice_timed"))
    check(elements().none { it.optString("label") == "Notification time remaining" })
    Thread.sleep(5200)
    check(call().getString("notice") == "Test error") { "An old timeout must not dismiss a new error" }
    call("feedback", "kind" to "long-error")
    Thread.sleep(400)
    val longNotice = element("Notification", "Groupbox")
    val longText = call().getString("notice")
    fun movingText() = elements().last { it.getString("label") == longText }
    check(movingText().getDouble("w") > longNotice.getDouble("w")) { "Full error text must be retained" }
    val before = movingText().getDouble("x")
    Thread.sleep(2400)
    check(movingText().getDouble("x") < before - 10) { "Long error must scroll automatically" }
    capture("scrolling-error")
    check(!call().getBoolean("notice_timed")) { "Errors must stay until dismissed" }
    call("feedback", "kind" to "dismiss")
    call("page", "value" to 0)
    note("animated scan indicator, notification gap and countdown in both themes, restart, timeout and smooth collapse")
}
