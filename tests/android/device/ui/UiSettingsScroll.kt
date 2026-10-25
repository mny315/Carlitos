package io.github.mny315.carlitos

import android.os.SystemClock
import android.view.MotionEvent
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun UiSuite.settingsScrollChecks() {
    shell("settings put system font_scale 1.0")
    eventually("Default font for scroll checks") { kotlin.math.abs(call().getDouble("font_scale") - 1) < .001 }
    for ((theme, name) in listOf(0 to "dark", 1 to "light")) {
        call("page", "value" to 0)
        Thread.sleep(300)
        call("appearance", "theme" to theme)
        call("page", "value" to 3)
        Thread.sleep(400)
        val scale = call().getDouble("scale")
        fun buttonColor(): Int {
            val button = element("English")
            val screen = instrumentation.uiAutomation.takeScreenshot()!!
            return try {
                screen.getPixel(((button.getDouble("x") + button.getDouble("w") - 8) * scale).toInt(),
                    ((button.getDouble("y") + button.getDouble("h") / 2) * scale).toInt())
            } finally { screen.recycle() }
        }
        val normal = buttonColor()
        val button = element("English")
        val x = ((button.getDouble("x") + button.getDouble("w") / 2) * scale).toFloat()
        val y = ((button.getDouble("y") + button.getDouble("h") / 2) * scale).toFloat()
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, y, down)
        Thread.sleep(180)
        pointer(MotionEvent.ACTION_MOVE, x, y - (40 * scale).toFloat(), down)
        Thread.sleep(200)
        capture("settings-scroll-$name-held")
        check(element("English").getDouble("y") < button.getDouble("y") - 10) { "Gesture must scroll settings" }
        check(buttonColor() == normal) { "Scrolling must clear the button highlight before release ($name)" }
        pointer(MotionEvent.ACTION_UP, x, y - (40 * scale).toFloat(), down)
        Thread.sleep(350)
        check(buttonColor() == normal) { "Scrolling must not leave a highlighted button ($name)" }
        check(call().getInt("overlay") == 0) { "Scrolling must not activate the button" }

        // Start at the numeric field's border while it owns text-input focus.
        val input = element("Text size (%)", "Spinbox")
        tapAt(input.getDouble("x") + 22, input.getDouble("y") + input.getDouble("h") / 2)
        capture("settings-scroll-$name-focused")
        eventually("Numeric field receives input focus") { call().getBoolean("keyboard") }
        val field = element("Text size (%)", "Spinbox")
        val fieldX = ((field.getDouble("x") + 5) * scale).toFloat()
        val fieldY = ((field.getDouble("y") + 3) * scale).toFloat()
        val fieldDown = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, fieldX, fieldY, fieldDown)
        Thread.sleep(180)
        pointer(MotionEvent.ACTION_MOVE, fieldX, fieldY - (40 * scale).toFloat(), fieldDown)
        eventually("Scrolling clears text-input focus before release") { !call().getBoolean("keyboard") }
        capture("settings-scroll-$name-unfocused")
        val unfocusedY = element("Text size (%)", "Spinbox").getDouble("y")
        pointer(MotionEvent.ACTION_MOVE, fieldX, fieldY - (80 * scale).toFloat(), fieldDown)
        Thread.sleep(150)
        check(element("Text size (%)", "Spinbox").getDouble("y") < unfocusedY - 10) {
            "The same gesture must continue scrolling after the keyboard closes"
        }
        pointer(MotionEvent.ACTION_UP, fieldX, fieldY - (80 * scale).toFloat(), fieldDown)
        check(call().getInt("text_size_percent") == 100) { "Scrolling must not edit the focused value" }
    }
    call("setup")
    call("page", "value" to 0)
    note("settings scroll clears touch highlights and input focus before release in both themes")
}
