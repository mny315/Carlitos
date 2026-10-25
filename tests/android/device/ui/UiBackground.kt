package io.github.mny315.carlitos

import android.content.Intent
import android.os.Process
import android.os.SystemClock
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.util.UnstableApi
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder

@UnstableApi
internal fun UiSuite.backgroundChecks(title: String) {
    // A lifecycle checkpoint must not resurrect an idle service after Android
    // releases it. In background, startService is forbidden and used to leak
    // its raw exception into the notice banner.
    eventually("Initial audio service") { Bridge.service != null }
    main { Bridge.service!!.stopSelf() }
    eventually("Idle service released") { Bridge.service == null }
    call("feedback", "kind" to "dismiss")
    shell("input keyevent KEYCODE_HOME")
    Thread.sleep(700)
    Bridge.dispatch(JSONObject().put("op", "snapshot").toString())
    Thread.sleep(500)
    check(Bridge.service == null) { "Checkpoint must not start an idle service" }
    main { instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    eventually("Return from idle background") { call().getBoolean("ui_visible") }
    Thread.sleep(500)
    check(call().getString("notice").isEmpty()) { "Idle checkpoint leaked a service-start error: ${call()}" }
    check(Bridge.service == null) { "Resuming without audio must not restart the service" }
    note("idle service remains stopped across background/foreground checkpoints with no error banner")

    // The observer must never consume, reorder or rewrite the sink's buffer.
    val pcm = ByteBuffer.allocateDirect(4096).order(ByteOrder.BIG_ENDIAN)
    repeat(2048) { pcm.put((it * 2), 0); pcm.put(it * 2 + 1, 8) }
    pcm.position(8); pcm.limit(4088)
    val original = ByteArray(pcm.remaining()).also { pcm.duplicate().get(it) }
    VoiceLevel.configure(Format.Builder().setPcmEncoding(C.ENCODING_PCM_16BIT).setChannelCount(2).setSampleRate(48000).build())
    VoiceLevel.enabled = true; VoiceLevel.foreground = true
    VoiceLevel.observe(pcm)
    check(VoiceLevel.read() > 0f)
    check(pcm.position() == 8 && pcm.limit() == 4088 && pcm.order() == ByteOrder.BIG_ENDIAN)
    check(ByteArray(pcm.remaining()).also { pcm.duplicate().get(it) }.contentEquals(original))
    val sampled = VoiceLevel.sampleCount
    VoiceLevel.observe(pcm)
    check(VoiceLevel.sampleCount == sampled) { "PCM sampling must be rate-limited" }
    VoiceLevel.enabled = false
    Thread.sleep(100)
    VoiceLevel.observe(pcm)
    check(VoiceLevel.sampleCount == sampled && VoiceLevel.read() == 0f)
    note("voice observation preserves PCM bytes and buffer ownership, throttles and disables")

    for ((band, frequency) in listOf(90.0, 450.0, 1200.0, 3600.0).withIndex()) {
        val tone = ByteBuffer.allocateDirect(4096).order(ByteOrder.LITTLE_ENDIAN)
        repeat(2048) { frame -> tone.putShort((kotlin.math.sin(frame * 2 * Math.PI * frequency / 48000) * 3932).toInt().toShort()) }
        tone.flip()
        VoiceLevel.configure(Format.Builder().setPcmEncoding(C.ENCODING_PCM_16BIT).setChannelCount(1).setSampleRate(48000).build())
        VoiceLevel.enabled = true
        VoiceLevel.observe(tone)
        val packed = VoiceLevel.readBands()
        val levels = (0..3).map { ((packed shr (it * 8)) and 255).toInt() }
        check(levels[band] > 0 && levels.withIndex().all { it.index == band || it.value < levels[band] }) {
            "$frequency Hz must drive its own voice band: $levels"
        }
        check(tone.position() == 0) { "Frequency analysis must not consume audio" }
        VoiceLevel.enabled = false
        check(VoiceLevel.readBands() == 0L)
    }
    note("four frequency groups respond independently, without consuming PCM or sampling while disabled")

    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }.first { it.getString("title") == title }.getString("key")
    call("action", "name" to "open-book", "arg" to book)
    val part = call().getJSONArray("part_rows").getJSONObject(0).getString("key").substringBefore(':').toLong()
    val playback = PlaybackSuite(instrumentation)
    fun audio(): JSONObject = playback.call("state").getJSONObject("playback")
    call("background", "enabled" to true, "reduced" to false)
    Bridge.emit("volume", "value" to 0.0)
    playback.call("part", "id" to part, "position" to 0)
    eventually("Voice-reactive background starts") { call().getBoolean("background_running") && VoiceLevel.read() > 0f }
    Thread.sleep(800)
    val before = nativeBackdropFrames()
    Thread.sleep(1200)
    val frames = nativeBackdropFrames() - before
    check(frames in 8..17) { "Background exceeded its frame budget: $frames in 1.2 s" }
    capture("background-dark")
    call("appearance", "theme" to 1)
    Thread.sleep(800)
    capture("background-light")
    call("appearance", "theme" to 0)

    fun stopped(description: String) {
        Thread.sleep(250)
        val frame = nativeBackdropFrames()
        val sample = VoiceLevel.sampleCount
        Thread.sleep(1000)
        check(nativeBackdropFrames() == frame && VoiceLevel.sampleCount == sample) {
            "$description kept visualizer work alive: frames=$frame->${nativeBackdropFrames()}, samples=$sample->${VoiceLevel.sampleCount}, foreground=${VoiceLevel.foreground}, enabled=${VoiceLevel.enabled}"
        }
    }
    call("background", "enabled" to false)
    eventually("Background disabled") { !call().getBoolean("background_running") }
    stopped("Disabled setting")
    val settings = JSONObject(java.io.File(instrumentation.targetContext.filesDir, "settings.json").readText())
    check(!settings.getBoolean("animated_background")) { "Background setting must persist" }
    call("background", "enabled" to true, "reduced" to true)
    eventually("Reduced motion disables background") { !call().getBoolean("background_running") }
    stopped("Reduced motion")
    call("background", "reduced" to false)
    eventually("Background resumes") { call().getBoolean("background_running") }
    Bridge.emit("play", "value" to false)
    eventually("Paused background stops") { !call().getBoolean("background_running") }
    stopped("Pause")
    Bridge.emit("play", "value" to true)
    eventually("Playback resumes") { call().getBoolean("background_running") }

    shell("input keyevent KEYCODE_HOME")
    stopped("Background Activity")
    main { instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    eventually("Visible background resumes") { call().getBoolean("background_running") && VoiceLevel.foreground }
    val position = audio().getLong("position")
    shell("input keyevent KEYCODE_SLEEP")
    val power = instrumentation.targetContext.getSystemService(android.content.Context.POWER_SERVICE) as android.os.PowerManager
    eventually("Screen actually turns off") { !power.isInteractive }
    eventually("Screen-off event disables voice observation") { !VoiceLevel.foreground }
    stopped("Locked screen")
    check(!VoiceLevel.foreground && VoiceLevel.read() == 0f)
    check(audio().getLong("position") > position + 800) { "Screen-off playback must continue" }
    shell("input keyevent KEYCODE_WAKEUP")
    shell("wm dismiss-keyguard")
    main { instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    eventually("Unlock resumes background") { call().getBoolean("background_running") }

    // Same decoder, silent output and screen for both samples; report process
    // CPU separately from wall time, without polling UI during measurement.
    fun cpu(enabled: Boolean): Double {
        call("background", "enabled" to enabled)
        Thread.sleep(1000)
        val cpu = Process.getElapsedCpuTime()
        val start = SystemClock.elapsedRealtime()
        Thread.sleep(4000)
        return (Process.getElapsedCpuTime() - cpu) * 100.0 / (SystemClock.elapsedRealtime() - start)
    }
    val off1 = cpu(false)
    val on = cpu(true)
    val off2 = cpu(false)
    note("background CPU, percent of one core: off=${"%.2f".format((off1 + off2) / 2)}, on=${"%.2f".format(on)}")
    call("background", "enabled" to true)
    note("voice-reactive background at <=12.5 fps; setting persistence; zero visualizer frames and samples while disabled, reduced, paused, in background and screen off; audio continues")
}
