package io.github.mny315.carlitos

import android.app.Instrumentation
import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import kotlin.math.abs

/** Exercises the real Rust controller, Store, JNI, MediaSession and ExoPlayer. */
@UnstableApi
class PlaybackSuite(internal val instrumentation: Instrumentation) {
    private external fun nativeRequest(request: String): String
    internal lateinit var controller: MediaController
    private val base = "content://io.github.mny315.carlitos.test.playback/document/"
    internal fun main(action: () -> Unit) = instrumentation.runOnMainSync(action)
    internal fun call(op: String, vararg fields: Pair<String, Any>): JSONObject {
        val request = JSONObject().put("op", op)
        fields.forEach { (key, value) -> request.put(key, value) }
        return JSONObject(nativeRequest(request.toString())).also { check(!it.has("error")) { it.toString() } }
    }
    internal fun note(message: String) = instrumentation.sendStatus(0, Bundle().apply { putString("stream", "PASS $message\n") })
    internal fun eventually(message: String, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 10000
        while (SystemClock.elapsedRealtime() < deadline) {
            if (condition()) return
            Thread.sleep(30)
        }
        var clock = ""
        main {
            clock = "controller=${controller.currentPosition}, playing=${controller.isPlaying}, state=${controller.playbackState}"
        }
        error("$message ($clock): ${call("state")}")
    }
    internal fun playerCheck(condition: () -> Boolean): Boolean {
        var result = false
        main { result = condition() }
        return result
    }
    internal fun position(): Long { var value = 0L; main { value = controller.currentPosition }; return value }
    internal fun sourcePosition(): Long {
        var value = 0L
        main {
            val player = PlaybackService::class.java.getDeclaredField("player").apply { isAccessible = true }
                .get(Bridge.service) as androidx.media3.exoplayer.ExoPlayer
            value = player.currentPosition
        }
        return value
    }
    internal fun audioBarrier() {
        val done = java.util.concurrent.CountDownLatch(1)
        main {
            val player = PlaybackService::class.java.getDeclaredField("player").apply { isAccessible = true }
                .get(Bridge.service) as androidx.media3.exoplayer.ExoPlayer
            val handler = android.os.Handler(player.applicationLooper)
            player.createMessage { _, _ -> handler.post { done.countDown() } }.send()
        }
        check(done.await(2, TimeUnit.SECONDS)) { "Audio thread did not acknowledge pause" }
    }
    internal fun pause() {
        main { controller.pause() }
        eventually("Pause") {
            val playback = call("state").getJSONObject("playback")
            // MediaController applies pause optimistically before Media3 and
            // Rust acknowledge it. Wait for the source clock to converge too.
            playerCheck { !controller.playWhenReady && abs(controller.currentPosition - playback.getLong("position")) < 150 } &&
                !playback.getBoolean("playing")
        }
        // The initial pause event is optimistic. Wait for the audio-thread
        // acknowledgement before using the final system-controller clock.
        audioBarrier()
        eventually("Confirmed pause") {
            val state = call("state")
            val position = position()
            sourcePosition() == position && state.getJSONObject("playback").getLong("position") == position &&
                state.getJSONObject("saved").getLong("position") == position
        }
    }
    internal fun play() {
        main { controller.play() }
        eventually("Play") { playerCheck { controller.isPlaying } }
    }
    internal fun seek(position: Long) {
        eventually("Seek command available") {
            playerCheck { controller.isCommandAvailable(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM) } &&
                call("state").getJSONObject("playback").getString("phase") == "Ready"
        }
        main { controller.seekTo(position) }
        eventually("Seek $position") {
            val playback = call("state").getJSONObject("playback")
            abs(position() - position) < 150 && playback.getString("phase") == "Ready" &&
                abs(playback.getLong("position") - position) < 150
        }
    }
    internal fun selected(id: Long): Boolean = call("state").getJSONObject("session").optJSONObject("current")?.optLong("Book") == id
    internal fun load(id: Long, position: Long = 0) {
        call("part", "id" to id, "position" to position)
        eventually("Load $id") { selected(id) && call("state").getJSONObject("playback").getString("phase") == "Ready" && playerCheck { controller.isPlaying } }
    }
    internal fun rate(rate: Float) {
        main { controller.setPlaybackSpeed(rate) }
        eventually("Rate $rate") { abs(call("state").getJSONObject("settings").getDouble("playback_rate") - rate) < 0.001 }
    }
    internal fun silence(value: Boolean) {
        call("silence", "value" to value)
        eventually("Silence $value") { call("state").getJSONObject("settings").getBoolean("skip_silence") == value }
    }
    internal fun mode(value: String) {
        instrumentation.targetContext.contentResolver.call(Uri.parse(base + "fault"), "mode", "fault", Bundle().apply { putString("value", value) })
    }
    internal fun fixture(title: String, ids: List<String>): List<Long> {
        val files = JSONArray()
        ids.forEach { id ->
            files.put(JSONObject().put("id", 0).put("source_id", 0).put("uri", base + id)
                .put("relative", "$id.wav").put("identity", "playback:$id").put("size", 5760044)
                .put("modified", 1).put("duration", 60000).put("title", id).put("artist", "Device suite")
                .put("album", title).put("track", JSONObject.NULL).put("disc", JSONObject.NULL)
                .put("cover", JSONObject.NULL).put("sort_tags_read", true)
                .put("chapters", JSONArray().put(JSONObject().put("title", "First").put("start", 0).put("end", 20000))
                    .put(JSONObject().put("title", "Second").put("start", 20000).put("end", 60000))))
        }
        val state = call("fixture", "root" to (base + title), "title" to title, "files" to files)
        val books = state.getJSONArray("books")
        val book = (0 until books.length()).map { books.getJSONObject(it) }.first { it.getString("title") == title }.getLong("id")
        val parts = state.getJSONArray("parts")
        return (0 until parts.length()).map { parts.getJSONObject(it) }.filter { it.getLong("book_id") == book }
            .sortedBy { it.getInt("ordinal") }.map { it.getLong("id") }
    }
}
