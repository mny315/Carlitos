package io.github.mny315.carlitos

import android.media.AudioDeviceInfo
import android.media.AudioTrack
import android.os.SystemClock
import android.view.MotionEvent
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.BaseRenderer
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.MediaCodecAudioRenderer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.abs

/** Exercise touch transport without tapAt's intentional 350 ms settling delay. */
@UnstableApi
internal fun UiSuite.pauseResponsivenessChecks(requireBluetooth: Boolean = false) {
    val playback = PlaybackSuite(instrumentation)
    val parts = listOf("tone", "silence").map { id ->
        val state = playback.call("state")
        val media = state.getJSONArray("media")
        val file = (0 until media.length()).map { media.getJSONObject(it) }
            .firstOrNull { it.getString("uri").endsWith("/$id") }
        if (file == null) playback.fixture("Pause timing $id", listOf(id)).single()
        else {
            val rows = state.getJSONArray("parts")
            (0 until rows.length()).map { rows.getJSONObject(it) }
                .first { it.getLong("file_id") == file.getLong("id") }.getLong("id")
        }
    }
    lateinit var player: ExoPlayer
    lateinit var sink: AudioSink
    lateinit var renderer: androidx.media3.exoplayer.Renderer
    var outputDetails = ""
    main {
        // Only the instrumented test inspects the actual output clock. A
        // MediaController masks play/pause before the audio thread handles it.
        player = PlaybackService::class.java.getDeclaredField("player").apply { isAccessible = true }
            .get(Bridge.service) as ExoPlayer
        renderer = (0 until player.rendererCount).map { player.getRenderer(it) }
            .first { it.trackType == C.TRACK_TYPE_AUDIO }
        sink = MediaCodecAudioRenderer::class.java.getDeclaredField("audioSink").apply { isAccessible = true }
            .get(renderer) as AudioSink
    }
    fun position(): Long { var result = 0L; main { result = player.currentPosition }; return result }
    fun playing(): Boolean { var result = false; main { result = player.playWhenReady }; return result }
    fun outputPosition(): Long {
        val done = CountDownLatch(1)
        var result: Result<Long>? = null
        main { player.createMessage { _, _ ->
            result = runCatching {
                val offset = BaseRenderer::class.java.getDeclaredField("streamOffsetUs").apply { isAccessible = true }.getLong(renderer)
                val position = (sink.getCurrentPositionUs(false) - offset) / 1000
                val track = DefaultAudioSink::class.java.getDeclaredField("audioTrack").apply { isAccessible = true }.get(sink) as AudioTrack
                check(track.playState == AudioTrack.PLAYSTATE_PAUSED) { "AudioTrack did not actually pause" }
                outputDetails = "mode=${track.performanceMode} underruns=${track.underrunCount} frames=${track.bufferSizeInFrames} rate=${track.sampleRate} channels=${track.channelCount} route=${track.routedDevice?.type}"
                if (requireBluetooth) check(track.routedDevice?.type in listOf(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
                    AudioDeviceInfo.TYPE_BLE_HEADSET, AudioDeviceInfo.TYPE_BLE_SPEAKER)) {
                    "Bluetooth pause check used another output: $outputDetails"
                }
                position
            }
            done.countDown()
        }.send() }
        check(done.await(2, TimeUnit.SECONDS)) { "Audio thread did not acknowledge transport" }
        return result!!.getOrThrow()
    }
    fun press(label: String): Long {
        val e = element(label)
        val scale = call().getDouble("scale")
        val x = ((e.getDouble("x") + e.getDouble("w") / 2) * scale).toFloat()
        val y = ((e.getDouble("y") + e.getDouble("h") / 2) * scale).toFloat()
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, y, down)
        val release = SystemClock.elapsedRealtime()
        pointer(MotionEvent.ACTION_UP, x, y, down)
        return release
    }
    try {
        playback.call("load_settings_burst", "id" to parts[0], "position" to 5000, "rate" to 1.75, "silence" to true)
        eventually("Changing settings immediately before loading preserves both settings") {
            val state = playback.call("state").getJSONObject("playback")
            state.getString("phase") == "Ready" && abs(state.getDouble("rate") - 1.75) < .001 && state.getBoolean("silence")
        }
        note("rate and silence changes queued immediately before a load are preserved")
        for ((index, part) in parts.withIndex()) {
            for (speed in listOf(1.0, 1.75, 2.0, 3.0)) {
                Bridge.emit("rate", "value" to speed)
                playback.call("silence", "value" to (index == 1))
                eventually("Pause timing playback settings") {
                    val settings = playback.call("state").getJSONObject("settings")
                    abs(settings.getDouble("playback_rate") - speed) < .001 && settings.getBoolean("skip_silence") == (index == 1)
                }
                playback.call("part", "id" to part, "position" to 5000)
                eventually("Pause timing fixture starts") {
                    playing() && playback.call("state").getJSONObject("playback").getString("phase") == "Ready"
                }
                main { player.volume = 0f }
                Thread.sleep(1100)
                val before = position()
                val release = press("Pause")
                val deadline = release + 2000
                while (playing() && SystemClock.elapsedRealtime() < deadline) Thread.sleep(5)
                check(!playing()) { "Touch pause did not reach Media3" }
                val output = outputPosition()
                val elapsed = SystemClock.elapsedRealtime() - release
                val paused = position()
                Thread.sleep(700)
                val settled = position()
                val saved = playback.call("state").getJSONObject("saved").getLong("position")
                var mediaId: String? = null
                main { mediaId = player.currentMediaItem?.mediaId }
                val resume = press("Play")
                while (!playing() && SystemClock.elapsedRealtime() < resume + 2000) Thread.sleep(5)
                check(playing()) { "Touch resume did not reach Media3" }
                Thread.sleep(80)
                val resumed = position()
                var resumedMediaId: String? = null
                main { resumedMediaId = player.currentMediaItem?.mediaId }
                note("pause timing silence=${index == 1} speed=$speed touch=${elapsed}ms before=$before paused=$paused output=$output settled=$settled saved=$saved resumed=$resumed resumeElapsed=${SystemClock.elapsedRealtime() - resume}ms $outputDetails")
                check(elapsed < 250) { "Touch pause took ${elapsed}ms" }
                check(abs(settled - paused) < 50) { "Paused source clock moved: $paused -> $settled" }
                check(saved == settled) { "Pause checkpoint differs from the settled source clock: $saved / $settled" }
                check(mediaId == resumedMediaId) { "Resume reloaded the media item" }
                if (index == 0) check(resumed >= settled && resumed - settled <= (SystemClock.elapsedRealtime() - resume + 50) * speed) {
                    "Resume moved away from the paused position: $settled -> $resumed"
                }
                main { player.pause() }
            }
        }
    } finally {
        Bridge.emit("play", "value" to false)
        Bridge.emit("rate", "value" to 1.0)
        playback.call("silence", "value" to false)
    }
}
