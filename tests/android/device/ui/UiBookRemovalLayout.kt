package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.floor

@UnstableApi
internal fun UiSuite.bookRemovalLayoutChecks() {
    val fixture = PlaybackSuite(instrumentation)
    val prefix = "00 Removal layout "
    repeat(30) { fixture.fixture(prefix + it.toString().padStart(2, '0'), listOf("removal-layout-$it")) }
    call("page", "value" to 0)
    tap("All")
    fun books(): List<JSONObject> = call().getJSONArray("books").let { rows ->
        (0 until rows.length()).map { rows.getJSONObject(it) }
    }
    eventually("Long removal fixture library") { books().count { it.getString("title").startsWith(prefix) } == 30 }
    val keys = books().filter { it.getString("title").startsWith(prefix) }.map { it.getString("key") }
    fun row(title: String) = elements().first { it.getString("label").startsWith("$title · ") }
    fun filledBottom() {
        val area = element("Books", "TabPanel")
        val bottom = area.getDouble("y") + area.getDouble("h")
        val rows = elements().filter { it.getString("label").contains(" · Device suite") }
        check(rows.any { it.getDouble("y") <= bottom && it.getDouble("y") + it.getDouble("h") >= bottom - 1 }) {
            "Long library left blank space at the bottom: $rows vs $bottom"
        }
    }
    try {
        eventually("Initial list settled") { !call().getBoolean("books_moving") }
        val height = call().getDouble("book_row_height")
        call("scroll-books", "y" to -7 * height)
        Thread.sleep(100)
        val removed = books()[10]
        val follower = books()[11].getString("title")
        val before = row(follower).getDouble("y")
        val scroll = call().getDouble("books_scroll_y")
        call("action", "name" to "remove-book", "arg" to removed.getString("key"))
        eventually("Middle removal starts movement") { call().getBoolean("books_moving") }
        var intermediate = false
        repeat(8) {
            val y = row(follower).getDouble("y")
            intermediate = intermediate || (y < before - 1 && y > before - height + 1)
            filledBottom()
            Thread.sleep(30)
        }
        eventually("Middle removal settled") { !call().getBoolean("books_moving") }
        check(intermediate) { "Remaining books jumped instead of sliding into the removed row" }
        check(abs(row(follower).getDouble("y") - (before - height)) < 1)
        check(abs(call().getDouble("books_scroll_y") - scroll) < 1) { "Middle removal changed the reading position" }
        filledBottom()
        val oldLimit = (call().getDouble("books_viewport_height") - books().size * height).coerceAtMost(0.0)
        call("scroll-books", "y" to oldLimit)
        Thread.sleep(100)
        val target = books()[floor(-oldLimit / height).toInt() + 1]
        call("action", "name" to "remove-book", "arg" to target.getString("key"))
        eventually("Bottom removal starts movement") { call().getBoolean("books_moving") }
        repeat(8) { filledBottom(); Thread.sleep(30) }
        eventually("Bottom removal settled") { !call().getBoolean("books_moving") }
        check(abs(call().getDouble("books_scroll_y") - (oldLimit + height)) < 1) {
            "Deleting near the end must clamp scroll to the shortened list: ${call()}"
        }
        filledBottom()
        capture("book-removal-filled-bottom")
        note("long-list deletion slides following books, preserves middle scroll and fills the bottom at the end")
    } finally {
        keys.forEach { key -> call("action", "name" to "remove-book", "arg" to key) }
        call("scroll-books", "y" to 0)
    }
}
