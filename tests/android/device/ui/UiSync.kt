package io.github.mny315.carlitos

import android.content.res.Configuration
import android.graphics.Bitmap
import android.os.SystemClock
import android.view.MotionEvent
import androidx.media3.common.util.UnstableApi
import com.journeyapps.barcodescanner.CaptureActivity
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import java.io.File

/** Real touch, scrolling, keyboard and camera Activity, in the isolated APK. */
@UnstableApi
internal fun UiSuite.scrollSyncTo(label: String) {
    repeat(14) {
        val state = call()
        val height = state.getDouble("height")
        // The accessibility tree omits controls outside the scroll viewport.
        val item = elements().lastOrNull { it.getString("label") == label }
        val y = item?.getDouble("y") ?: height
        if (item != null && y >= 130 && y + item.getDouble("h") < height - 190) return
        val scale = state.getDouble("scale").toFloat()
        val x = (state.getDouble("width") - 18).toFloat() * scale
        val upper = (height * .25).toFloat() * scale
        val lower = (height * .65).toFloat() * scale
        val from = if (y < 130) upper else lower
        val to = if (y < 130) lower else upper
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, from, down)
        repeat(12) { step ->
            Thread.sleep(20)
            pointer(MotionEvent.ACTION_MOVE, x, from + (to - from) * (step + 1) / 12, down)
        }
        pointer(MotionEvent.ACTION_UP, x, to, down)
        Thread.sleep(350)
    }
    error("Cannot scroll to $label")
}

@UnstableApi
internal fun UiSuite.syncButtonChecks() {
    for (page in listOf(0, 3)) {
        call("page", "value" to page)
        Thread.sleep(350)
        if (page == 3) scrollSyncTo("Sync")
        val button = element("Sync")
        check(button.getDouble("h") >= 48) { "Sync touch target shrank: $button" }
        val scale = call().getDouble("scale").toFloat()
        val x = (button.getDouble("x") + button.getDouble("w") / 2).toFloat() * scale
        val y = (button.getDouble("y") + button.getDouble("h") / 2).toFloat() * scale
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, y, down)
        pointer(MotionEvent.ACTION_UP, x, y, down)
        eventually("Sync button shows activity on page $page") { call().getBoolean("sync_busy") }
        check(element("Syncing…").getDouble("h") >= 48)
        instrumentation.uiAutomation.takeScreenshot().let { bitmap ->
            File(output, "sync-running-page-$page.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
        eventually("One-sided sync completes on page $page") {
            val state = call()
            !state.getBoolean("sync_busy") && state.getString("sync_message") == "Synchronization complete"
        }
        capture("sync-complete-page-$page")
    }
    scrollSyncTo("Synchronization")
    tap("Synchronization", "Checkbox")
    eventually("Synchronization disabled") { !call().getBoolean("sync_enabled") }
    call("page", "value" to 0)
    Thread.sleep(350)
    check(elements().none { it.getString("label") in listOf("Sync", "Syncing…") }) {
        "Disabled synchronization left a library button"
    }
    capture("sync-disabled-library")
    call("page", "value" to 3)
    Thread.sleep(350)
    scrollSyncTo("Synchronization")
    tap("Synchronization", "Checkbox")
    eventually("Synchronization re-enabled") { call().getBoolean("sync_enabled") }
    note("Library and settings sync buttons: activity, completion and 48dp touch targets")
    note("Disabling synchronization hides its library button and preserves pairing")
}

@UnstableApi
internal fun UiSuite.syncAppearanceChecks() {
    val packageName = instrumentation.targetContext.packageName
    val originalRotation = shell("settings get system user_rotation")
    val originalAuto = shell("settings get system accelerometer_rotation")
    try {
        shell("input keyevent KEYCODE_WAKEUP")
        Thread.sleep(800)
        shell("wm dismiss-keyguard")
        shell("settings put system accelerometer_rotation 0")
        shell("settings put system user_rotation 0")
        eventually("Sync settings received") { !call().getBoolean("loading") && call().getBoolean("sync_enabled") }
        call("appearance", "theme" to 0, "language" to 2)
        call("page", "value" to 3)
        eventually("Settings surface and controls ready") {
            call().getDouble("width") > 200 && call().getDouble("height") > 300
        }
        Thread.sleep(600)
        scrollSyncTo("Pair using a code")
        capture("sync-enabled-dark")
        tap("Pair using a code")
        check(call().getBoolean("sync_manual"))
        scrollSyncTo("Pair")
        val input = element("Device code", "TextInput")
        val pair = element("Pair")
        check(pair.getDouble("x") >= input.getDouble("x") + input.getDouble("w")) {
            "Pair button overlaps the code input"
        }
        capture("sync-manual-dark")
        tap("Device code", "TextInput")
        eventually("Pairing input keyboard") { call().getBoolean("keyboard") }
        instrumentation.sendStringSync("invalid-" + "a".repeat(320))
        capture("sync-manual-keyboard")
        back()
        scrollSyncTo("Pair")
        tap("Pair")
        eventually("Invalid pairing code reported") { call().getString("sync_message").isNotEmpty() }
        capture("sync-invalid-code")
        scrollSyncTo("Pair using a code")
        tap("Pair using a code")
        scrollSyncTo("Show pairing QR code")
        tap("Show pairing QR code")
        eventually("Pairing QR visible") { call().getBoolean("sync_invitation") }
        scrollSyncTo("Carlitos pairing QR code")
        capture("sync-qr-dark")
        call("appearance", "theme" to 1, "language" to 1)
        scrollSyncTo("QR-код подключения Carlitos")
        capture("sync-qr-light-russian")
        scrollSyncTo("Закрыть QR-код")
        tap("Закрыть QR-код")
        eventually("QR cancelled") { !call().getBoolean("sync_invitation") }
        scrollSyncTo("Подключение по коду")
        tap("Подключение по коду")
        scrollSyncTo("Подключить")
        capture("sync-manual-light-russian")
        scrollSyncTo("Подключение по коду")
        tap("Подключение по коду")
        scrollSyncTo("Сканировать QR")
        shell("pm grant $packageName android.permission.CAMERA")
        val monitor = instrumentation.addMonitor(CaptureActivity::class.java.name, null, false)
        tap("Сканировать QR")
        val camera = checkNotNull(instrumentation.waitForMonitorWithTimeout(monitor, 8000)) as CaptureActivity
        try {
            val deadline = SystemClock.elapsedRealtime() + 8000
            var ready = false
            while (!ready && SystemClock.elapsedRealtime() < deadline) {
                main { ready = camera.hasWindowFocus() && camera.findViewById<DecoratedBarcodeView>(com.google.zxing.client.android.R.id.zxing_barcode_scanner).barcodeView.isPreviewActive }
                Thread.sleep(100)
            }
            check(ready) { "Camera preview did not start" }
            main { check(camera.resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT) {
                "QR camera turned the portrait phone into landscape"
            } }
            instrumentation.uiAutomation.takeScreenshot().let { bitmap ->
                File(output, "sync-camera-portrait.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
            }
            instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            eventually("Settings restored after camera") { runCatching { call().getInt("page") == 3 }.getOrDefault(false) }
            capture("sync-return-from-camera")
        } finally { instrumentation.removeMonitor(monitor) }
        call("appearance", "theme" to 0, "language" to 2)
        note("Sync settings, manual input, QR, dark/light themes, Russian/English and live portrait camera")
    } finally {
        shell(if (originalRotation == "null") "settings delete system user_rotation" else "settings put system user_rotation $originalRotation")
        shell(if (originalAuto == "null") "settings delete system accelerometer_rotation" else "settings put system accelerometer_rotation $originalAuto")
    }
}
