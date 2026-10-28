package io.github.mny315.carlitos

import android.content.Intent
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import androidx.media3.common.util.UnstableApi
import kotlin.math.abs

/** Background source clocks across speed, silence and Activity boundaries. */
@UnstableApi
internal fun PlaybackSuite.backgroundPowerChecks() {
    fun shell(command: String) {
        instrumentation.uiAutomation.executeShellCommand(command).use {
            ParcelFileDescriptor.AutoCloseInputStream(it).use { input -> input.readBytes() }
        }
    }
    fun foreground() {
        shell("input keyevent KEYCODE_WAKEUP")
        Thread.sleep(500)
        shell("wm dismiss-keyguard")
        main { instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext,
            MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        eventually("Foreground visibility") { playerCheck { VoiceLevel.foreground } }
    }
    fun background() {
        shell("input keyevent KEYCODE_HOME")
        shell("input keyevent KEYCODE_SLEEP")
        eventually("Background visibility") { playerCheck { !VoiceLevel.foreground } }
        Thread.sleep(1800)
    }
    fun clock(speed: Float) {
        Thread.sleep(750)
        val before = sourcePosition()
        val start = SystemClock.elapsedRealtime()
        Thread.sleep(2000)
        val elapsed = SystemClock.elapsedRealtime() - start
        val advanced = sourcePosition() - before
        check(abs(advanced - elapsed * speed) < 350) { "Background $speed clock: $advanced / $elapsed" }
        pause()
        val output = outputDetails()
        check(output.getInt("underruns") == 0) { "Background $speed output underrun: $output" }
        check(outputDetails().getInt("play_state") == android.media.AudioTrack.PLAYSTATE_PAUSED)
        val paused = position()
        Thread.sleep(300)
        check(position() == paused && sourcePosition() == paused) { "Paused background clock moved" }
        note("background $speed clock $advanced ms / $elapsed ms; exact pause and saved checkpoint")
        play()
    }
    foreground()
    val mp3 = fixture("Background compressed audio", listOf("mp3-long")).single()
    val silent = fixture("Background silence", listOf("silence")).single()
    main { controller.volume = 0f }
    rate(1f); silence(false); load(mp3, 5000)
    note("foreground compressed output=${outputDetails()}")
    val foregroundFrames = outputDetails().getInt("frames")
    background()
    val details = outputDetails()
    check(details.getInt("frames") == foregroundFrames) { "Background changed the output queue: $details" }
    if (android.os.Build.VERSION.SDK_INT >= 31) check(details.getInt("start_frames") <= foregroundFrames)
    note("background compressed output=$details")
    clock(1f)
    for (speed in listOf(1.75f, .5f, 3f)) {
        rate(speed)
        clock(speed)
    }
    rate(1f)
    // Real PCM processing in background must still use the source clock.
    load(silent); pause(); silence(true); seek(0); play()
    Thread.sleep(2400); pause()
    check(position() > 4000) { "Background silence removal stopped working: ${position()}" }
    silence(false); load(mp3, 10000)
    background()
    foreground()
    check(outputDetails().getInt("frames") == foregroundFrames)
    clock(1f)
    pause()
    note("background playback preserves output latency, silence removal and exact checkpoints")
}
