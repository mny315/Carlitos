package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import android.content.pm.ActivityInfo
import android.os.SystemClock
import android.view.KeyEvent
import android.view.MotionEvent
import org.json.JSONObject
import java.io.File

@UnstableApi
internal fun UiSuite.displayScaleChecks() {
    shell("settings put system font_scale 1.0")
    eventually("Default system font") { kotlin.math.abs(call().getDouble("font_scale") - 1) < .001 }
    call("page", "value" to 3)
    Thread.sleep(400)
    val density = call().getDouble("scale")
    drag("Text size", .2f, .4f) {
        val live = call()
        check(live.getInt("text_size_percent") in 120..130)
        check(kotlin.math.abs(live.getDouble("font_scale") - live.getInt("text_size_percent") / 100.0) < .001) { "Text must resize before release" }
    }
    val textPercent = call().getInt("text_size_percent")
    check(textPercent in 120..130)
    check(kotlin.math.abs(call().getDouble("font_scale") - textPercent / 100.0) < .001)
    check(kotlin.math.abs(call().getDouble("scale") - density) < .001)
    capture("settings-text-slider")
    tap("Reset: Text size")
    percentSteps("Text size", "text_size_percent", 205)
    tap("Reset: Text size")
    check(element("Text size", "Slider").getDouble("value") == 100.0) { "Reset must also move the thumb" }
    tap("Text size (%)", "Spinbox")
    instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_MOVE_END)
    repeat(3) { instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_DEL) }
    instrumentation.sendStringSync("123")
    capture("settings-exact-input")
    eventually("Exact text size") { call().getInt("text_size_percent") == 123 }
    back()
    shell("settings put system font_scale 1.25")
    eventually("Custom and system text scales compose") { kotlin.math.abs(call().getDouble("font_scale") - 1.5375) < .001 }
    shell("settings put system font_scale 1.0")
    eventually("System font changes preserve custom text size") { kotlin.math.abs(call().getDouble("font_scale") - 1.23) < .001 }
    tap("Reset: Text size")
    percentSteps("Interface scale", "ui_scale_percent", 160)
    tap("Reset: Interface scale")
    drag("Interface scale", .3f, .6f) {
        val live = call()
        check(live.getInt("ui_scale_percent") in 115..130)
        check(kotlin.math.abs(live.getDouble("scale") - density * live.getInt("ui_scale_percent") / 100.0) < .001) { "Interface must resize before release" }
    }
    val uiPercent = call().getInt("ui_scale_percent")
    check(uiPercent in 115..130)
    val scaledDensity = density * uiPercent / 100
    check(kotlin.math.abs(call().getDouble("scale") - scaledDensity) < .001)
    check(kotlin.math.abs(call().getDouble("font_scale") - 1) < .001)
    capture("settings-interface-slider")
    eventually("Display settings persisted") {
        val saved = JSONObject(File(instrumentation.targetContext.filesDir, "settings.json").readText())
        kotlin.math.abs(saved.getDouble("android_ui_scale") - uiPercent / 100.0) < .001
    }
    main { Bridge.activity.get()!!.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
    eventually("Scaled landscape") { call().getDouble("width") > call().getDouble("height") }
    check(kotlin.math.abs(call().getDouble("scale") - scaledDensity) < .001)
    main { Bridge.activity.get()!!.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
    eventually("Scaled portrait") { call().getDouble("height") > call().getDouble("width") }
    var previous: MainActivity? = null
    main { previous = Bridge.activity.get(); previous!!.recreate() }
    eventually("Scaled Activity recreated") {
        var ready = false
        main { ready = Bridge.activity.get()?.let { it !== previous && it.hasWindowFocus() } == true }
        ready && runCatching { call().getInt("ui_scale_percent") == uiPercent }.getOrDefault(false)
    }
    check(kotlin.math.abs(call().getDouble("scale") - scaledDensity) < .001)
    call("setup")
    call("page", "value" to 0)
    note("live text/interface resizing at 15 percent ticks, stable hold, exact entry, persistence and recreation")
}

@UnstableApi
internal fun UiSuite.percentSteps(label: String, field: String, maximum: Int) {
    val e = element(label, "Slider")
    val scale = call().getDouble("scale")
    val origin = e.getDouble("x") + 10
    val width = e.getDouble("w") - 20
    val y = ((e.getDouble("y") + e.getDouble("h") / 2) * scale).toFloat()
    fun x(value: Int) = ((origin + width * (value - 70) / (maximum - 70)) * scale).toFloat()
    val down = SystemClock.uptimeMillis()
    pointer(MotionEvent.ACTION_DOWN, x(100), y, down)
    // ScrollView waits 100 ms to distinguish a tap from scrolling.
    Thread.sleep(150)
    val ticks = call().getInt("haptic_ticks")
    for ((position, percent) in listOf(106 to 100, 109 to 115, 117 to 115, 130 to 130, 115 to 115, 100 to 100)) {
        pointer(MotionEvent.ACTION_MOVE, x(position), y, down)
        Thread.sleep(65)
        check(call().getInt(field) == percent) { "$label must snap to 15% ticks: ${call()}" }
    }
    check(call().getInt("haptic_ticks") == ticks + 4) { "Vibrate only when crossing a notch" }
    for (percent in listOf(maximum, 70, 100)) {
        pointer(MotionEvent.ACTION_MOVE, x(percent), y, down)
        Thread.sleep(150)
        check(call().getInt(field) == percent) { "$label lost a full-range drag: ${call()}" }
        val heldTicks = call().getInt("haptic_ticks")
        repeat(4) {
            pointer(MotionEvent.ACTION_MOVE, x(percent), y, down)
            Thread.sleep(80)
            check(call().getInt(field) == percent && call().getInt("haptic_ticks") == heldTicks) {
                "$label oscillates under a stationary finger: ${call()}"
            }
        }
    }
    pointer(MotionEvent.ACTION_UP, x(100), y, down)
    Thread.sleep(300)
    check(call().getInt(field) == 100) { "Release must retain the previewed value" }
}
