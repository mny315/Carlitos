package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import kotlin.math.abs

@UnstableApi
internal fun PlaybackSuite.silencePauseClock(silent: Long) {
    load(silent); pause(); silence(true)
    main { controller.volume = 0f }
    repeat(4) {
        rate(if (it % 2 == 0) 1.25f else 1.75f)
        seek(0); play(); Thread.sleep(3200); pause()
        val source = position()
        eventually("Repeated silence pause $it at $source") {
            abs(call("state").getJSONObject("saved").getLong("position") - source) < 150
        }
    }
    note("repeated silence pauses keep the system controller and saved source clock aligned")
}
@UnstableApi
internal fun PlaybackSuite.pauseSettingsBursts(tone: Long) {
    load(tone)
    main { controller.volume = 0f }
    repeat(21) { index ->
        seek(5000)
        play()
        Thread.sleep(300)
        val speed = listOf(.5f, 1f, 1.75f, 3f)[index % 4]
        main {
            controller.pause()
            controller.setPlaybackSpeed(speed)
        }
        call("silence", "value" to (index % 2 == 0))
        eventually("Pause/settings burst $index reaches Rust") {
            val playback = call("state").getJSONObject("playback")
            !playback.getBoolean("playing") && abs(playback.getDouble("rate") - speed) < .001 &&
                playback.getBoolean("silence") == (index % 2 == 0)
        }
        audioBarrier()
        eventually("Pause/settings burst $index saves the settled source clock (${sourcePosition()})") {
            val source = sourcePosition()
            val state = call("state")
            state.getJSONObject("saved").getLong("position") == source &&
                state.getJSONObject("playback").getLong("position") == source && position() == source
        }
        val paused = sourcePosition()
        Thread.sleep(100)
        check(sourcePosition() == paused) { "Pause/settings burst $index moved while paused" }
    }
    rate(1f)
    silence(false)
    note("rapid pause/speed/silence commands preserve the exact settled checkpoint")
}
