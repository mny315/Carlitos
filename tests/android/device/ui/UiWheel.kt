package io.github.mny315.carlitos

import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import androidx.media3.common.util.UnstableApi
import kotlin.math.abs

@UnstableApi
internal fun UiSuite.mouseWheelChecks() {
    Bridge.emit("volume", "value" to .4)
    eventually("Wheel fixture volume") { abs(call().getDouble("volume") - .4) < .001 }
    tap("Volume")
    val slider = element("Volume", "Slider")
    val scale = call().getDouble("scale")
    fun wheel(axis: Int, delta: Float) {
        val properties = MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE }
        val coordinates = MotionEvent.PointerCoords().apply {
            x = ((slider.getDouble("x") + slider.getDouble("w") / 2) * scale).toFloat()
            y = ((slider.getDouble("y") + slider.getDouble("h") / 2) * scale).toFloat()
            setAxisValue(axis, delta)
        }
        val now = SystemClock.uptimeMillis()
        val event = MotionEvent.obtain(now, now, MotionEvent.ACTION_SCROLL, 1, arrayOf(properties),
            arrayOf(coordinates), 0, 0, 1f, 1f, 0, 0, InputDevice.SOURCE_MOUSE, 0)
        try { check(instrumentation.uiAutomation.injectInputEvent(event, true)) }
        finally { event.recycle() }
    }
    wheel(MotionEvent.AXIS_VSCROLL, 1f)
    eventually("Vertical mouse wheel increases volume") { call().getDouble("volume") > .425 }
    val raised = call().getDouble("volume")
    wheel(MotionEvent.AXIS_HSCROLL, -1f)
    eventually("Horizontal mouse wheel decreases volume") { call().getDouble("volume") < raised - .025 }
    back()
    note("vertical and horizontal mouse wheel input reaches controls without crashing")
}
