package io.github.mny315.carlitos

import android.os.SystemClock
import android.view.MotionEvent
import androidx.media3.common.util.UnstableApi
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

@UnstableApi
internal fun UiSuite.bookSelectionChecks() {
    fun document(op: String, id: String, expected: JSONObject? = null): JSONObject = JSONObject(Documents.request(JSONObject()
        .put("op", op).put("uri", "content://io.github.mny315.carlitos.test.sources/" +
            (if (id == "delete-readonly") "" else "tree/delete-root/") + "document/$id")
        .put("expected", expected ?: JSONObject.NULL).toString()))
    check(document("check-delete", "delete-readonly").has("error"))
    check(document("stat", "delete-readonly").has("value"))
    check(document("check-delete", "delete-audio").getBoolean("value"))
    val unchanged = JSONObject().put("size", 16044).put("modified", 1234000)
    check(document("check-delete", "delete-audio", unchanged).getBoolean("value"))
    for (changed in listOf(JSONObject().put("size", 16045), JSONObject().put("modified", 1234001))) {
        for (op in listOf("check-delete", "delete")) {
            check(document(op, "delete-audio", changed).has("error"))
            check(document("stat", "delete-audio").has("value"))
        }
    }
    check(document("delete", "delete-error").has("error"))
    check(document("stat", "delete-error").has("value"))
    check(document("delete", "delete-audio", unchanged).getBoolean("value"))
    check(document("stat", "delete-audio").has("error"))
    check(document("delete", "delete-audio", unchanged).getBoolean("value"))
    note("SAF write permission, changed-file protection, idempotent deletion, read-only access and provider failure")
    val titles = (1..3).map { "00 Book selection $it" }
    val files = titles.mapIndexed { index, title ->
        val root = File(instrumentation.targetContext.filesDir, "book-selection/$index").apply { mkdirs() }
        val file = File(root, "part.wav").apply { writeText("Book audio $index") }
        val metadata = android.system.Os.stat(file.absolutePath)
        val media = JSONObject().put("id", 0).put("source_id", 0).put("uri", file.toURI().toString())
            .put("relative", file.name).put("identity", "${metadata.st_dev}:${metadata.st_ino}").put("size", metadata.st_size)
            .put("modified", metadata.st_mtime).put("duration", 60000).put("title", "Part").put("artist", "Device suite")
            .put("album", title).put("track", JSONObject.NULL).put("disc", JSONObject.NULL)
            .put("cover", JSONObject.NULL).put("sort_tags_read", true).put("chapters", JSONArray())
        PlaybackSuite(instrumentation).call("fixture", "root" to root.toURI().toString(),
            "title" to title, "files" to JSONArray().put(media), "folder" to true)
        file
    }
    fun books(): List<JSONObject> = call().getJSONArray("books").let { rows ->
        (0 until rows.length()).map { rows.getJSONObject(it) }
    }
    fun hold(title: String, cancel: Boolean = false, duration: Long = 650) {
        eventually("Book movement settled before holding") { !call().getBoolean("books_moving") }
        val area = element("Books", "TabPanel")
        val row = elements().first { it.getString("label").startsWith("$title · ") &&
            it.getDouble("y") >= area.getDouble("y") && it.getDouble("y") + it.getDouble("h") <= area.getDouble("y") + area.getDouble("h") }
        val scale = call().getDouble("scale")
        val x = ((row.getDouble("x") + row.getDouble("w") * .4) * scale).toFloat()
        val y = ((row.getDouble("y") + row.getDouble("h") * .5) * scale).toFloat()
        val down = SystemClock.uptimeMillis()
        pointer(MotionEvent.ACTION_DOWN, x, y, down)
        Thread.sleep(duration)
        pointer(if (cancel) MotionEvent.ACTION_CANCEL else MotionEvent.ACTION_UP, x, y, down)
        Thread.sleep(350)
    }
    fun selectTwo() {
        hold(titles[0])
        check(call().getBoolean("book_selection") && call().getInt("selected_book_count") == 1 && call().getInt("page") == 0)
        tap("Select book: ${titles[1]}", "Checkbox")
        check(call().getInt("selected_book_count") == 2)
    }
    call("page", "value" to 0)
    call("appearance", "theme" to 0)
    tap("All")
    tap("Playback speed")
    val sheetBottom = element("Done").let { it.getDouble("y") + it.getDouble("h") }
    back()
    fun checkConfirmationPosition() {
        val button = element("Remove")
        check(kotlin.math.abs(button.getDouble("y") + button.getDouble("h") - sheetBottom) <= 10) {
            "Removal confirmation must share the playback sheets' bottom inset: $button vs $sheetBottom"
        }
        check(button.getDouble("h") >= 48 && element("Cancel").getDouble("h") >= 48)
    }
    eventually("Selection book fixtures") { books().count { it.getString("title") in titles } == 3 }
    val keys = books().filter { it.getString("title") in titles }.map { it.getString("key") }
    try {
        hold(titles[0], cancel = true, duration = 150)
        check(!call().getBoolean("book_selection") && call().getInt("page") == 0)
        hold(titles[0], duration = 100)
        check(call().getInt("page") == 1 && !call().getBoolean("book_selection"))
        back()
        selectTwo()
        tap("Select all books")
        check(call().getInt("selected_book_count") == books().size)
        tap("Deselect books")
        check(call().getInt("selected_book_count") == 0)
        tap("Select book: ${titles[0]}", "Checkbox")
        tap("Select book: ${titles[1]}", "Checkbox")
        capture("books-selection-dark")
        val selectionGeneration = call().getLong("book_selection_generation")
        recreate()
        eventually("Book selection restored") { call().getInt("selected_book_count") == 2 && call().getBoolean("book_selection") }
        check(call().getLong("book_selection_generation") == selectionGeneration)
        call("feedback", "kind" to "books-changed", "generation" to selectionGeneration)
        check(!call().getBoolean("book_selection") && call().getInt("selected_book_count") == 0)
        selectTwo()
        tap("Selected book actions")
        back()
        check(call().getBoolean("book_selection") && !call().getBoolean("menu"))
        tap("Selected book actions")
        tap("Remove from library")
        check(call().getInt("overlay") == 1 && call().getInt("remove_book_count") == 2 && !call().getBoolean("remove_book_files"))
        checkConfirmationPosition()
        tap("Delete book files", "Checkbox")
        capture("books-remove-confirmation-dark")
        tap("Cancel")
        check(files.all { it.exists() } && call().getInt("selected_book_count") == 2)
        tap("Selected book actions")
        tap("Remove from library")
        check(!call().getBoolean("remove_book_files"))
        tapAt(call().getDouble("width") / 2, 100.0)
        check(call().getInt("overlay") == 0 && call().getInt("selected_book_count") == 2 && files.all { it.exists() })
        tap("Selected book actions")
        tap("Remove from library")
        back()
        back()
        check(!call().getBoolean("book_selection") && call().getInt("page") == 0)
        selectTwo()
        tap("Selected book actions")
        tap("Mark as completed")
        eventually("Selected books completed") { !call().getBoolean("book_selection") && books().count { it.getString("title") in titles.take(2) && it.getBoolean("completed") } == 2 }
        selectTwo()
        tap("Selected book actions")
        tap("Mark as unstarted")
        eventually("Selected progress reset") { !call().getBoolean("book_selection") && books().none { it.getString("title") in titles.take(2) && it.getBoolean("started") } }
        call("appearance", "theme" to 1)
        selectTwo()
        capture("books-selection-light")
        tap("Selected book actions")
        tap("Remove from library")
        checkConfirmationPosition()
        capture("books-remove-confirmation-light")
        tap("Remove")
        eventually("Batch removal from library") { !call().getBoolean("book_selection") && books().none { it.getString("title") in titles.take(2) } }
        check(files.all { it.exists() })
        hold(titles.last())
        tap("Selected book actions")
        tap("Remove from library")
        tap("Delete book files", "Checkbox")
        tap("Remove")
        eventually("Opt-in file deletion") { books().none { it.getString("title") == titles.last() } }
        check(!files.last().exists() && files.take(2).all { it.exists() })
        note("book long press, taps, cancellation, select all, recreation, Back, batch completion/reset/removal and opt-in audio deletion")
    } finally {
        call("action", "name" to "clear-book-selection")
        keys.forEach { key -> call("action", "name" to "remove-book", "arg" to key) }
        call("appearance", "theme" to 2)
        call("page", "value" to 0)
    }
}
