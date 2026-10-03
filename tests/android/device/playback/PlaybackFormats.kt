package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import org.json.JSONObject
import java.util.concurrent.TimeUnit

@UnstableApi
internal fun PlaybackSuite.outputDetails(): JSONObject {
    val done = java.util.concurrent.CountDownLatch(1)
    var output: Result<JSONObject>? = null
    main {
        val player = PlaybackService::class.java.getDeclaredField("player").apply { isAccessible = true }
            .get(Bridge.service) as androidx.media3.exoplayer.ExoPlayer
        val renderer = (0 until player.rendererCount).map { player.getRenderer(it) }
            .first { it.trackType == androidx.media3.common.C.TRACK_TYPE_AUDIO }
        player.createMessage { _, _ ->
            output = runCatching {
                val sink = androidx.media3.exoplayer.audio.MediaCodecAudioRenderer::class.java
                    .getDeclaredField("audioSink").apply { isAccessible = true }.get(renderer)
                val track = androidx.media3.exoplayer.audio.DefaultAudioSink::class.java
                    .getDeclaredField("audioTrack").apply { isAccessible = true }.get(sink) as android.media.AudioTrack
                JSONObject().put("rate", track.sampleRate).put("channels", track.channelCount)
                    .put("mode", track.performanceMode).put("underruns", track.underrunCount)
                    .put("frames", track.bufferSizeInFrames)
            }
            done.countDown()
        }.send()
    }
    check(done.await(2, TimeUnit.SECONDS)) { "Audio thread stalled" }
    return output!!.getOrThrow()
}
@UnstableApi
internal fun PlaybackSuite.formatCompatibilityChecks(playMs: Long) {
    DocumentChecks(instrumentation).largeAudioPacket()
    main { controller.volume = 0f }
    silence(false)
    for (name in listOf("pcm-16000", "pcm-44100", "pcm-96000", "mp3-mono", "mp3-stereo",
        "aac", "m4b", "he-aac-v1", "he-aac-v2", "flac", "flac-large")) {
        val part = fixture("Output format $name", listOf(name)).single()
        // The default compressed fixtures are one second long. Longer
        // playback intervals require the corresponding extended assets.
        rate(if (name.startsWith("pcm-")) 1.75f else .5f)
        val interval = when {
            name.startsWith("pcm-") -> playMs.coerceAtMost(1500)
            name == "flac-large" -> playMs.coerceAtMost(500)
            else -> playMs
        }
        load(part)
        Thread.sleep(interval)
        pause()
        eventually("Exact pause checkpoint for $name") {
            call("state").getJSONObject("saved").getLong("position") == position()
        }
        val paused = position()
        Thread.sleep(150)
        check(position() == paused) { "$name advanced while paused" }
        val beforeResume = outputDetails()
        check(beforeResume.getInt("underruns") == 0) { "$name output underruns: $beforeResume" }
        play(); Thread.sleep(interval); pause()
        val details = outputDetails()
        check(details.getInt("underruns") == 0) { "$name resumed output underruns: $details" }
        check(position() > paused) { "$name did not resume from the paused position" }
        check(call("state").getJSONObject("playback").isNull("error"))
        note("output $name pause/checkpoint/resume $details")
    }
    rate(1f)
}
