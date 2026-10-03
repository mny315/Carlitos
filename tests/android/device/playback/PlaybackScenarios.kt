package io.github.mny315.carlitos

import android.content.ComponentName
import android.content.Intent
import android.os.SystemClock
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import java.util.concurrent.TimeUnit
import kotlin.math.abs

@UnstableApi
internal fun PlaybackSuite.run(formatsOnly: Boolean = false, formatPlayMs: Long = 150) {
    require(formatPlayMs in 150..10000)
    check(instrumentation.targetContext.packageName.endsWith(".playbacktest"))
    instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    lateinit var future: com.google.common.util.concurrent.ListenableFuture<MediaController>
    main { future = MediaController.Builder(instrumentation.targetContext,
        SessionToken(instrumentation.targetContext, ComponentName(instrumentation.targetContext, PlaybackService::class.java))).buildAsync() }
    controller = future.get(10, TimeUnit.SECONDS)
    try {
        check(playerCheck { controller.seekBackIncrement == 15000L && controller.seekForwardIncrement == 15000L }) {
            "System seek increments must match the routed ±15 s commands"
        }
        mode("normal")
        val parts = fixture("Parts", listOf("tone", "part2"))
        val silent = fixture("Silence", listOf("silence")).single()
        val fault = fixture("Fault", listOf("fault")).single()
        call("busy_document_import")
        note("single-document import cannot bypass a source update or relocation")
        formatCompatibilityChecks(formatPlayMs)
        if (formatsOnly) return
        failedServiceStart(fault, false)
        failedServiceStart(fault, true)
        checkpointWhileLoading(fault)
        silencePauseClock(silent)
        pauseSettingsBursts(parts[0])
        load(parts[0]); pause(); silence(false)
        main { controller.volume = 0f }
        for (speed in listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f)) {
            rate(speed); seek(5000); play()
            // Start after the output has settled, then measure source time.
            Thread.sleep(250)
            val before = position(); val time = SystemClock.elapsedRealtime()
            Thread.sleep(1300)
            val elapsed = SystemClock.elapsedRealtime() - time
            val advanced = position() - before
            check(abs(advanced - elapsed * speed) < 400) { "Rate $speed: $advanced / $elapsed" }
            pause(); note("speed $speed source clock: $advanced ms / $elapsed ms")
        }
        rate(1f)
        repeat(2) { call("rate_step", "value" to true) }
        eventually("Rate step +0.05") { abs(call("state").getJSONObject("settings").getDouble("playback_rate") - 1.1) < 0.001 }
        call("rate_step", "value" to false)
        eventually("Rate step -0.05") { abs(call("state").getJSONObject("settings").getDouble("playback_rate") - 1.05) < 0.001 }
        for ((edge, forward) in listOf(0.5f to false, 3f to true)) {
            rate(edge); call("rate_step", "value" to forward)
            eventually("Rate boundary") { abs(call("state").getJSONObject("playback").getDouble("rate") - edge) < 0.001 }
        }
        rate(1f)
        main { listOf(4000L, 22000L, 7000L, 16000L).forEach { controller.seekTo(it) } }
        eventually("Rapid seek final checkpoint") { call("state").getJSONObject("saved").getLong("position") == 16000L }
        // Chapters use the same Part(id, source_position) command as the UI.
        load(parts[0], 20000); pause()
        check(position() in 20000..21000)
        seek(55000)
        main { controller.seekForward() }
        eventually("Seek forward across parts") {
            selected(parts[1]) && abs(position() - 10000) < 150 &&
                playerCheck { controller.isCommandAvailable(Player.COMMAND_SEEK_BACK) }
        }
        main { controller.seekBack() }
        eventually("Seek backward across parts") {
            selected(parts[0]) && abs(position() - 55000) < 150 &&
                playerCheck { controller.isCommandAvailable(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM) }
        }
        main { controller.seekToNextMediaItem() }
        eventually("Next paused part checkpoint") {
            selected(parts[1]) && call("state").getJSONObject("saved").getJSONObject("current").getLong("Book") == parts[1] &&
                playerCheck { controller.isCommandAvailable(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM) }
        }
        main { controller.seekToPreviousMediaItem() }
        eventually("Previous paused part") { selected(parts[0]) && position() < 150 && playerCheck { !controller.playWhenReady } }
        note("chapters, ±15 s across parts, next/previous and rapid seek checkpoint")

        seek(12000); play(); Thread.sleep(600)
        val beforeSwitch = call("state").getJSONObject("session").getLong("position")
        load(silent); pause()
        val switched = call("state").getJSONArray("saved_progress")
        check((0 until switched.length()).map { switched.getJSONObject(it) }
            .any { it.getLong("part_id") == parts[0] && it.getLong("position") >= beforeSwitch })
        note("switching books checkpoints the previous book's confirmed live position")
        rate(1f); silence(false); seek(0); play()
        Thread.sleep(2400); pause(); val unskipped = position()
        silence(true); seek(0); play(); Thread.sleep(2400); pause(); val skipped = position()
        check(unskipped in 1800..3400 && skipped > unskipped + 1500) { "Silence: $unskipped -> $skipped" }
        val saved = call("state").getJSONObject("saved").getLong("position")
        check(abs(saved - skipped) < 300) { "Source clock not checkpointed after skip: $saved / $skipped" }
        rate(2f); seek(20000); play(); Thread.sleep(1500); pause()
        check(position() > 24000) { "Silence at 2x did not advance source time" }
        silence(false); seek(20000)
        check(call("state").getJSONObject("saved").getLong("position") == 20000L)
        note("silence removal at 1x/2x, source position, seek and pause: $unskipped -> $skipped ms")
        silence(true)
        for (speed in listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f)) {
            rate(speed); seek(0); play()
            val time = SystemClock.elapsedRealtime()
            Thread.sleep(3200); pause()
            val elapsed = SystemClock.elapsedRealtime() - time
            val source = position()
            check(source > elapsed * speed + 500) { "Silence at $speed: $source ms / $elapsed ms" }
            eventually("Skip checkpoint $speed (paused at $source)") { abs(call("state").getJSONObject("saved").getLong("position") - source) < 200 }
            seek(20000)
            eventually("Source seek after skip $speed") { call("state").getJSONObject("saved").getLong("position") == 20000L }
            note("silence at $speed: $source ms / $elapsed ms, original-position seek and checkpoint")
        }
        silence(false)

        rate(1f); load(parts[0], 59500)
        eventually("EOS next part") { selected(parts[1]) && playerCheck { controller.isPlaying } }
        pause(); seek(59500); play()
        eventually("Final EOS checkpoint") { call("state").getJSONArray("saved_progress").let { p -> (0 until p.length()).any { p.getJSONObject(it).getBoolean("completed") } } }
        eventually("Final EOS pauses") { playerCheck { !controller.playWhenReady } }
        play()
        eventually("System play restarts completed book") { selected(parts[0]) && position() < 2000 }
        pause(); note("automatic part transition, completed book, system-play restart")

        load(fault); pause(); seek(7000)
        for (failure in listOf("missing", "broken")) {
            mode(failure)
            call("part", "id" to fault, "position" to 25000)
            eventually("Error $failure") { call("state").getJSONObject("playback").getString("phase") == "Error" }
            check(call("state").getJSONObject("saved").getLong("position") == 7000L)
            mode("normal"); play(); pause()
            check(position() in 7000..8000) { "Retry lost the checkpoint" }
            seek(7000)
        }
        note("missing/corrupt content URI preserves 7000 ms and retries from checkpoint")


        load(parts[0]); pause(); rate(1.75f); silence(true); seek(8000)
        main { controller.volume = 0.25f }
        eventually("Volume") { abs(call("state").getJSONObject("saved").getDouble("volume") - 0.25) < 0.001 }
        call("mute")
        eventually("Mute") { playerCheck { controller.volume == 0f } && call("state").getJSONObject("saved").getBoolean("muted") }
        call("mute")
        eventually("Unmute") { playerCheck { abs(controller.volume - 0.25f) < 0.001f } }
        play()
        var service: PlaybackService? = null
        var previous: MainActivity? = null
        main { service = Bridge.service; previous = Bridge.activity.get(); check(previous != null); previous!!.recreate() }
        eventually("Activity recreation") { playerCheck { Bridge.activity.get() != null && Bridge.activity.get() !== previous && Bridge.service === service && controller.isPlaying } }
        main { Bridge.activity.get()!!.finishAndRemoveTask() }
        eventually("Removed recent task") { playerCheck {
            instrumentation.targetContext.getSystemService(android.app.ActivityManager::class.java).appTasks.isEmpty() &&
                Bridge.activity.get() == null && Bridge.service === service && controller.isPlaying
        } }
        val background = position()
        Thread.sleep(1500)
        check(position() > background + 1000)
        instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        check(playerCheck { Bridge.service === service && controller.isPlaying })
        note("removing the actual recent task preserves service and source clock; reopening keeps player")
        pause(); seek(8000)
        note("volume/mute checkpoint, Activity recreation, persisted 1.75x + silence + 8000 ms")
    } finally {
        mode("normal")
        main { controller.pause(); controller.release() }
    }
}
