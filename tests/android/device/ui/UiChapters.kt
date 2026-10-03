package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import org.json.JSONObject
import kotlin.math.abs

@UnstableApi
internal fun UiSuite.chapterPlaybackChecks(title: String) {
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }
        .first { it.getString("title") == title }.getString("key")
    call("action", "name" to "open-book", "arg" to book)
    Thread.sleep(400)
    val part = call().getJSONArray("part_rows").getJSONObject(0).getString("key").substringBefore(':').toLong()
    val playback = PlaybackSuite(instrumentation)
    fun audio(): JSONObject = playback.call("state").getJSONObject("playback")
    fun activeChapter(start: Long): Boolean {
        val rows = call().getJSONArray("part_rows")
        val active = (0 until rows.length()).map { rows.getJSONObject(it) }.filter { it.getBoolean("active") }
        return active.size == 1 && active[0].getString("key") == "$part:$start" && active[0].getBoolean("chapter")
    }
    fun hasButton(label: String) = elements().any { it.getString("role").contains("Button") && it.getString("label") == label }
    fun chapterTap(label: String) {
        // Each file has First/Second chapters. Pick the first file's button.
        val button = elements().first { it.getString("role").contains("Button") && it.getString("label") == label }
        tapAt(button.getDouble("x") + button.getDouble("w") / 2, button.getDouble("y") + button.getDouble("h") / 2)
    }
    playback.call("part", "id" to part, "position" to 7000)
    eventually("Playing chapter highlighted") {
        audio().getString("phase") == "Ready" && call().getBoolean("playing") && activeChapter(0) && hasButton("Pause: First")
    }
    capture("chapter-playing")
    chapterTap("Pause: First")
    eventually("Chapter button pauses") { !audio().getBoolean("playing") && !call().getBoolean("playing") && hasButton("Continue: First") }
    val paused = audio().getLong("position")
    check(paused >= 6500 && activeChapter(0)) { "Pause must keep the chapter and its position" }
    Thread.sleep(250)
    check(abs(audio().getLong("position") - paused) < 150) { "Paused chapter clock advanced" }
    capture("chapter-paused")
    chapterTap("Continue: First")
    eventually("Chapter button resumes") { audio().getBoolean("playing") && hasButton("Pause: First") }
    check(audio().getLong("position") >= paused - 150) { "Resume restarted the chapter" }
    chapterTap("Play: Second")
    eventually("Another chapter replaces the highlight and pause button") {
        audio().getString("phase") == "Ready" && audio().getLong("position") in 20000..24000 &&
            activeChapter(20000) && hasButton("Pause: Second") && !hasButton("Pause: First")
    }
    capture("chapter-switched")
    chapterTap("Pause: Second")
    eventually("Pause before seeking") { !audio().getBoolean("playing") }
    Bridge.emit("seek", "position" to 5000)
    eventually("Paused seek updates the active chapter") {
        audio().getString("phase") == "Ready" && activeChapter(0) && hasButton("Continue: First") && !hasButton("Continue: Second")
    }
    Bridge.emit("seek", "position" to 19500)
    eventually("Seek before chapter boundary") { audio().getString("phase") == "Ready" && audio().getLong("position") in 19350..19650 }
    chapterTap("Continue: First")
    eventually("Playback crossing a chapter boundary moves the pause button") { activeChapter(20000) && hasButton("Pause: Second") && !hasButton("Pause: First") }
    chapterTap("Pause: Second")
    eventually("Final chapter pause") { !audio().getBoolean("playing") }
    note("chapter highlighting, play/pause, resume without restarting, chapter selection, paused seeking and automatic chapter transition")
}
