package io.github.mny315.carlitos

import android.content.Intent
import android.os.Process
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun PlaybackSuite.notificationStopKeepsBook(part: Long, checkUi: Boolean) {
    load(part); pause(); seek(8000)
    val selectedBook = call("state").getJSONArray("parts").let { parts ->
        (0 until parts.length()).map { parts.getJSONObject(it) }
            .first { it.getLong("id") == part }.getLong("book_id").toString()
    }
    val ui = if (checkUi) UiSuite(instrumentation) else null
    fun checkPlayer() {
        eventually("Stopped book keeps its checkpoint") {
            val state = call("state")
            selected(part) && state.getJSONObject("saved").getJSONObject("current").getLong("Book") == part &&
                state.getJSONObject("playback").getLong("position") == 8000L &&
                !state.getJSONObject("playback").getBoolean("playing")
        }
        ui?.eventually("Bottom player retains the book and time") {
            val state = runCatching { ui.call() }.getOrNull()
            state != null && state.getBoolean("active") && state.getString("current") == selectedBook &&
                state.getString("position_text") == "0:08" && !state.getBoolean("playing")
        }
    }
    main { controller.stop() }
    eventually("System stop releases the decoder") {
        call("state").getJSONObject("playback").getString("phase") == "Empty"
    }
    checkPlayer()
    val pid = Process.myPid()
    main { Bridge.activity.get()!!.finishAndRemoveTask() }
    eventually("Stopped Activity removed") { Bridge.activity.get() == null }
    instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    check(Process.myPid() == pid) { "Warm reopen must keep the same process" }
    checkPlayer()
    ui?.capture("last-book-after-stop-and-reopen")
    main { controller.play() }
    eventually("Play reloads the stopped book at its checkpoint") {
        selected(part) && playerCheck { controller.isPlaying } && position() in 8000..11000
    }
    pause(); seek(8000)
    note("notification Stop, Activity removal and warm reopen retain the bottom player; Play resumes from 8000 ms")
}
