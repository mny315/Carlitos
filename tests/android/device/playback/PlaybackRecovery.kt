package io.github.mny315.carlitos

import android.content.ComponentName
import android.content.ContextWrapper
import android.content.Intent
import android.os.SystemClock
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun PlaybackSuite.checkpointWhileLoading(fault: Long) {
    load(fault); pause(); seek(7000)
    mode("slow")
    call("part", "id" to fault, "position" to 25000)
    val checkpointStarted = SystemClock.elapsedRealtime()
    call("checkpoint")
    eventually("Checkpoint during a stalled document load") {
        call("state").getJSONObject("playback").getBoolean("barrier")
    }
    val checkpoint = call("state")
    check(SystemClock.elapsedRealtime() - checkpointStarted < 4000) { "Checkpoint waited for the blocked provider" }
    check(checkpoint.getJSONObject("playback").getString("phase") == "Loading")
    check(checkpoint.getJSONObject("saved").getLong("position") == 7000L)
    mode("normal")
    eventually("Delayed load eventually resumes") {
        call("state").getJSONObject("playback").getString("phase") == "Ready"
    }
    pause()
    note("stalled document load acknowledges checkpoint without saving an unconfirmed seek")
}
@UnstableApi
internal fun PlaybackSuite.failedServiceStart(fault: Long, timesOut: Boolean) {
    load(fault); pause(); seek(7000)
    val context = Bridge.context
    var service: PlaybackService? = null
    main {
        service = Bridge.service
        check(service != null)
        Bridge.service = null
        Bridge.context = object : ContextWrapper(context) {
            override fun startService(intent: Intent): ComponentName? {
                if (timesOut) return ComponentName(context, PlaybackService::class.java)
                throw IllegalStateException("Fixture: background service start denied")
            }
        }
    }
    try {
        call("part", "id" to fault, "position" to 25000)
        eventually("Service startup failure is retryable") {
            call("state").getJSONObject("playback").getString("phase") == "Error"
        }
        check(call("state").getJSONObject("saved").getLong("position") == 7000L)
    } finally {
        main { Bridge.context = context; Bridge.ready(service!!) }
    }
    Thread.sleep(300)
    check(position() == 7000L && playerCheck { !controller.playWhenReady }) {
        "A failed startup retained an obsolete load command"
    }
    play(); pause()
    check(position() in 7000..8000) { "Service retry lost the confirmed checkpoint" }
    note("service startup ${if (timesOut) "timeout" else "error"} clears queued loads and retries from the confirmed position")
}
