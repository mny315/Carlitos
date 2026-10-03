package io.github.mny315.carlitos

import android.app.Instrumentation
import android.graphics.Bitmap
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.media3.common.util.UnstableApi
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Reads Slint bounds, then sends actual Android touch and key events. */
@UnstableApi
internal class UiSuite(val instrumentation: Instrumentation) {
    private external fun nativeRequest(request: String): String
    internal val output get() = File(instrumentation.targetContext.getExternalFilesDir(null), "ui").apply { mkdirs() }
    fun main(action: () -> Unit) {
        var result: Result<Unit>? = null
        instrumentation.runOnMainSync { result = runCatching(action) }
        checkNotNull(result).getOrThrow()
    }
    fun call(op: String = "state", vararg fields: Pair<String, Any>): JSONObject {
        val request = JSONObject().put("op", op)
        fields.forEach { (key, value) -> request.put(key, value) }
        return JSONObject(nativeRequest(request.toString())).also { check(!it.has("error")) { it.toString() } }
    }
    fun elements(): List<JSONObject> {
        val raw = nativeRequest("{\"op\":\"elements\"}")
        check(raw.startsWith("[")) { raw }
        val array = JSONArray(raw)
        return (0 until array.length()).map { array.getJSONObject(it) }
    }
    fun shell(command: String): String = instrumentation.uiAutomation.executeShellCommand(command).use {
        ParcelFileDescriptor.AutoCloseInputStream(it).use { input -> String(input.readBytes()).trim() }
    }
    fun note(message: String) = instrumentation.sendStatus(0, Bundle().apply { putString("stream", "PASS $message\n") })
    fun eventually(message: String, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 12000
        while (SystemClock.elapsedRealtime() < deadline) {
            if (condition()) return
            Thread.sleep(80)
        }
        error("$message: ${call()}")
    }
    fun element(label: String, role: String = "Button"): JSONObject {
        val state = call()
        return elements().lastOrNull {
            it.getString("label") == label && it.getString("role").contains(role) &&
                it.getDouble("w") > 0 && it.getDouble("h") > 0 && it.getDouble("y") >= 0 &&
                it.getDouble("y") + it.getDouble("h") <= state.getDouble("height")
        } ?: error("No visible $role '$label': ${elements()}")
    }
    fun pointer(action: Int, x: Float, y: Float, down: Long) {
        val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0)
        event.source = android.view.InputDevice.SOURCE_TOUCHSCREEN
        val injected = instrumentation.uiAutomation.injectInputEvent(event, true)
        event.recycle()
        if (!injected) {
            capture("failed-touch")
            error("Touch injection failed: action=$action x=$x y=$y state=${call()}")
        }
    }
    internal fun cancelTouches() {
        val now = SystemClock.uptimeMillis()
        val event = MotionEvent.obtain(now, now, MotionEvent.ACTION_CANCEL, 0f, 0f, 0)
        event.source = android.view.InputDevice.SOURCE_TOUCHSCREEN
        instrumentation.uiAutomation.injectInputEvent(event, true)
        event.recycle()
    }
    fun tap(label: String, role: String = "Button") {
        val e = element(label, role)
        tapAt(e.getDouble("x") + e.getDouble("w") / 2, e.getDouble("y") + e.getDouble("h") / 2)
    }
    fun tapAt(logicalX: Double, logicalY: Double) {
        val scale = call().getDouble("scale")
        val x = (logicalX * scale).toFloat()
        val y = (logicalY * scale).toFloat()
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, y, down)
        pointer(MotionEvent.ACTION_UP, x, y, down)
        Thread.sleep(350)
    }
    fun drag(label: String, start: Float, end: Float, beforeRelease: (() -> Unit)? = null) {
        val e = element(label, "Slider")
        check(e.getDouble("h") >= 48) { "Small slider: $e" }
        val scale = call().getDouble("scale").toFloat()
        val x = e.getDouble("x").toFloat(); val width = e.getDouble("w").toFloat()
        val y = (e.getDouble("y") + e.getDouble("h") / 2).toFloat() * scale
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, (x + width * start) * scale, y, down)
        for (i in 1..12) {
            Thread.sleep(35)
            pointer(MotionEvent.ACTION_MOVE, (x + width * (start + (end - start) * i / 12)) * scale, y, down)
        }
        beforeRelease?.invoke()
        pointer(MotionEvent.ACTION_UP, (x + width * end) * scale, y, down)
        Thread.sleep(400)
    }
    fun back() { instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_BACK); Thread.sleep(450) }
    fun capture(name: String) {
        Thread.sleep(400)
        instrumentation.uiAutomation.takeScreenshot().let { bitmap ->
            File(output, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
        File(output, "$name.json").writeText(JSONObject().put("state", call()).put("elements", JSONArray(elements())).toString(2))
    }

}
