package io.github.mny315.carlitos

import android.content.Intent
import androidx.media3.common.util.UnstableApi
import org.json.JSONObject
import java.io.File

@UnstableApi
internal fun UiSuite.animationFrames() {
    check(instrumentation.targetContext.packageName.endsWith(".playbacktest"))
    shell("input keyevent KEYCODE_WAKEUP")
    Thread.sleep(800)
    shell("wm dismiss-keyguard")
    val activity = instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
    eventually("Animation window ready") { call().getDouble("height") > 300 }
    call("setup")
    call("animation-start")
    var report: JSONObject? = null
    val deadline = android.os.SystemClock.elapsedRealtime() + 65000
    while (report == null && android.os.SystemClock.elapsedRealtime() < deadline) {
        Thread.sleep(1000)
        val next = call("animation-report")
        if (!next.isNull("report")) report = next
    }
    val result = checkNotNull(report) { "Animation measurement did not complete" }
    main {
        @Suppress("DEPRECATION")
        val display = activity.windowManager.defaultDisplay
        result.put("display_hz", display.mode.refreshRate)
        result.put("requested_hz", activity.window.attributes.preferredRefreshRate)
        result.put("requested_mode", activity.window.attributes.preferredDisplayModeId)
        result.put("supported_max_hz", display.supportedModes.maxOf { it.refreshRate })
    }
    File(output, "animation-frames.json").writeText(result.toString(2))
    note("ANIMATION FRAMES $result")
}
