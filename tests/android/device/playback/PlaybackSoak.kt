package io.github.mny315.carlitos

import android.content.ComponentName
import android.content.Intent
import android.os.SystemClock
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

@UnstableApi
internal fun PlaybackSuite.soak(minutes: Int) {
    require(minutes in 1..120)
    check(instrumentation.targetContext.packageName.endsWith(".playbacktest"))
    instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    lateinit var future: com.google.common.util.concurrent.ListenableFuture<MediaController>
    main { future = MediaController.Builder(instrumentation.targetContext,
        SessionToken(instrumentation.targetContext, ComponentName(instrumentation.targetContext, PlaybackService::class.java))).buildAsync() }
    controller = future.get(10, TimeUnit.SECONDS)
    val samples = JSONArray()
    val output = java.io.File(instrumentation.targetContext.getExternalFilesDir(null), "playback-soak.json")
    val start = SystemClock.elapsedRealtime()
    var cycles = 0
    fun report() = output.writeText(JSONObject().put("minutes", minutes).put("cycles", cycles)
        .put("elapsed_ms", SystemClock.elapsedRealtime() - start).put("samples", samples).toString(2))
    try {
        mode("normal")
        val tone = fixture("Soak tone", listOf("tone")).single()
        val silent = fixture("Soak silence", listOf("silence")).single()
        main { controller.volume = 0f; Bridge.activity.get()?.moveTaskToBack(true) }
        instrumentation.uiAutomation.executeShellCommand("input keyevent KEYCODE_SLEEP").use {
            android.os.ParcelFileDescriptor.AutoCloseInputStream(it).use { input -> input.readBytes() }
        }
        val speeds = listOf(.5f, .75f, 1f, 1.25f, 1.75f, 2f, 3f)
        while (SystemClock.elapsedRealtime() - start < minutes * 60000L) {
            val speed = speeds[cycles % speeds.size]
            val skip = cycles % 2 == 1
            rate(speed); silence(skip)
            load(if (skip) silent else tone, (cycles % 3) * 5000L)
            Thread.sleep(1600)
            val pauseStarted = SystemClock.elapsedRealtime()
            pause()
            val paused = position()
            eventually("Soak exact checkpoint") {
                call("state").getJSONObject("saved").getLong("position") == position()
            }
            Thread.sleep(300)
            check(position() == paused) { "Soak paused clock moved at cycle $cycles ($speed, silence=$skip): $paused -> ${position()}, ${call("state")}" }
            play(); Thread.sleep(400); pause()
            check(position() >= paused) { "Soak resume moved backwards" }
            val state = call("state").getJSONObject("playback")
            check(state.isNull("error") && state.getString("phase") == "Ready") { state.toString() }
            cycles++
            if (cycles % 12 == 0) {
                val sample = JSONObject().put("cycle", cycles).put("speed", speed).put("silence", skip)
                    .put("position", position()).put("elapsed_ms", SystemClock.elapsedRealtime() - start)
                    .put("pss_kib", android.os.Debug.getPss())
                    .put("output", outputDetails())
                samples.put(sample)
                report()
                check(sample.getJSONObject("output").getInt("underruns") == 0) { "Soak output underruns: $sample" }
                note("screen-off soak cycle $cycles: $sample")
            }
            check(SystemClock.elapsedRealtime() - pauseStarted < 5000) { "Soak transport stalled" }
        }
        pause()
        report()
        note("screen-off playback soak completed: $cycles cycles in ${SystemClock.elapsedRealtime() - start}ms")
    } finally {
        report()
        main { controller.pause(); controller.release() }
    }
}
