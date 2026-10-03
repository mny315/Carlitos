package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import android.os.SystemClock
import android.view.MotionEvent
import kotlin.math.abs

@UnstableApi
private fun UiSuite.swipeAnimationChecks() {
    val area = element("Books", "TabPanel")
    val row = elements().first {
        it.getString("role").contains("Button") && it.getString("label").contains(" · Device suite") &&
            it.getDouble("y") >= area.getDouble("y") &&
            it.getDouble("y") + it.getDouble("h") <= area.getDouble("y") + area.getDouble("h")
    }
    val label = row.getString("label")
    val origin = row.getDouble("x")
    val scroll = call().getDouble("books_scroll_y")
    fun offset() = elements().first { it.getString("label") == label }.getDouble("x") - origin
    val scale = call().getDouble("scale").toFloat()
    val x = (row.getDouble("x") + row.getDouble("w") * .6).toFloat() * scale
    val y = (row.getDouble("y") + row.getDouble("h") * .5).toFloat() * scale
    val down = SystemClock.uptimeMillis()
    pointer(MotionEvent.ACTION_DOWN, x, y, down)
    var previous = 0f
    for (dx in listOf(-24f, -48f, -72f, -48f)) {
        // Feed a continuous trajectory: an instantaneous jump is extrapolated
        // by Android's touch predictor and does not match a finger's motion.
        for (step in 1..6) {
            Thread.sleep(8)
            pointer(MotionEvent.ACTION_MOVE, x + (previous + (dx - previous) * step / 6) * scale, y, down)
        }
        Thread.sleep(16)
        pointer(MotionEvent.ACTION_MOVE, x + dx * scale, y, down)
        Thread.sleep(30)
        check(call().getInt("page") == 0) { "Claiming a swipe must cancel the pending book tap" }
        // Android may resample the injected pointer slightly ahead of a frame.
        check(abs(offset() - dx * .3) < 3) { "Swipe animation must follow the finger, including reversal: dx=$dx offset=${offset()}" }
        previous = dx
    }
    val held = offset()
    Thread.sleep(220)
    check(abs(offset() - held) < .5) { "Holding a swipe still must not move the list" }
    pointer(MotionEvent.ACTION_CANCEL, x - 48f * scale, y, down)
    Thread.sleep(40)
    val returning = offset()
    check(returning > held && returning < -.2) { "Cancelled swipe must animate back without snapping: $held -> $returning" }
    Thread.sleep(250)
    check(abs(offset()) < .5 && call().getInt("filter") == 0 && call().getInt("page") == 0)
    check(abs(call().getDouble("books_scroll_y") - scroll) < .5) { "Horizontal dragging must not move vertical scroll" }
    note("swipe animation follows the finger, reverses, holds still and returns smoothly on cancellation")
}

@UnstableApi
internal fun UiSuite.swipeBooks(points: List<Pair<Float, Float>>, cancel: Boolean = false, captureDrag: Boolean = false, steps: Int = 12) {
    val area = element("Books", "TabPanel")
    val scale = call().getDouble("scale").toFloat()
    val filter = call().getInt("filter")
    fun x(point: Pair<Float, Float>) = (area.getDouble("x") + area.getDouble("w") * point.first).toFloat() * scale
    fun y(point: Pair<Float, Float>) = (area.getDouble("y") + area.getDouble("h") * point.second).toFloat() * scale
    val down = SystemClock.uptimeMillis()
    pointer(MotionEvent.ACTION_DOWN, x(points.first()), y(points.first()), down)
    points.zipWithNext().forEach { (start, end) ->
        for (i in 1..steps) {
            Thread.sleep(16)
            val point = (start.first + (end.first - start.first) * i / steps) to
                (start.second + (end.second - start.second) * i / steps)
            pointer(MotionEvent.ACTION_MOVE, x(point), y(point), down)
        }
    }
    check(call().getInt("filter") == filter) { "Filter changed before swipe release" }
    if (captureDrag) capture("library-swipe-drag")
    pointer(if (cancel) MotionEvent.ACTION_CANCEL else MotionEvent.ACTION_UP, x(points.last()), y(points.last()), down)
    Thread.sleep(400)
    check(call().getInt("page") == 0) { "Swipe opened a book" }
}

@UnstableApi
internal fun UiSuite.dragBookButton(label: String, dx: Float, dy: Float = 0f, cancel: Boolean = false, returnToStart: Boolean = false) {
    val before = call()
    val button = element(label)
    val scale = before.getDouble("scale").toFloat()
    val x = (button.getDouble("x") + button.getDouble("w") / 2).toFloat() * scale
    val y = (button.getDouble("y") + button.getDouble("h") / 2).toFloat() * scale
    val down = SystemClock.uptimeMillis()
    pointer(MotionEvent.ACTION_DOWN, x, y, down)
    for (i in 1..12) {
        Thread.sleep(16)
        pointer(MotionEvent.ACTION_MOVE, x + dx * scale * i / 12, y + dy * scale * i / 12, down)
    }
    if (returnToStart) for (i in 11 downTo 0) {
        Thread.sleep(16)
        pointer(MotionEvent.ACTION_MOVE, x + dx * scale * i / 12, y + dy * scale * i / 12, down)
    }
    check(call().getInt("filter") == before.getInt("filter")) { "Button swipe switched before release" }
    pointer(if (cancel) MotionEvent.ACTION_CANCEL else MotionEvent.ACTION_UP,
        x + if (returnToStart) 0f else dx * scale, y + if (returnToStart) 0f else dy * scale, down)
    Thread.sleep(450)
    val after = call()
    check(after.getInt("page") == 0 && !after.getBoolean("menu") &&
        after.getBoolean("playing") == before.getBoolean("playing")) { "Button drag activated a click: $after" }
}

@UnstableApi
internal fun UiSuite.bookSwipeChecks(playableTitle: String) {
    val fixture = PlaybackSuite(instrumentation)
    repeat(12) { fixture.fixture("Swipe fixture $it", listOf("swipe-$it")) }
    call("page", "value" to 0)
    tap("All")
    Thread.sleep(500)
    val fixtureKeys = call().getJSONArray("books").let { books ->
        (0 until books.length()).map { books.getJSONObject(it) }.filter { it.getString("title").startsWith("Swipe fixture ") }
            .map { it.getString("key") }
    }
    try {
        check(fixtureKeys.size == 12)
        swipeBooks(listOf(.7f to .15f, .25f to .23f), steps = 3)
        check(call().getInt("filter") == 1) { "Fast diagonal swipe over books must change filter" }
        swipeBooks(listOf(.25f to .15f, .7f to .23f), steps = 3)
        check(call().getInt("filter") == 0) { "Fast reverse swipe must return to All" }
        tap("All")
        swipeAnimationChecks()
        swipeBooks(listOf(.7f to .08f, .25f to .08f), captureDrag = true)
        check(call().getInt("filter") == 1) { "Swipe directly on a book must change filter" }
        tap("All")
        val before = call().getDouble("books_scroll_y")
        swipeBooks(listOf(.5f to .8f, .5f to .25f))
        check(call().getInt("filter") == 0 && call().getDouble("books_scroll_y") < before - 40) { "Vertical book scrolling must remain available" }
        swipeBooks(listOf(.7f to .3f, .25f to .32f))
        check(call().getInt("filter") == 1) { "Slightly diagonal swipe on a scrolled book must change filter" }
        tap("All")
        swipeBooks(listOf(.7f to .08f, .25f to .08f), cancel = true)
        check(call().getInt("filter") == 0)
        swipeBooks(listOf(.5f to .08f, .46f to .08f))
        check(call().getInt("filter") == 0)
        val row = elements().first { it.getString("role").contains("Button") && it.getString("label").contains(" · Device suite") && it.getDouble("y") > 0 }
        tapAt(row.getDouble("x") + row.getDouble("w") * .4, row.getDouble("y") + row.getDouble("h") * .5)
        check(call().getInt("page") == 1) { "A tap must still open the book" }
        back()
        check(call().getInt("page") == 0) { "Back from a tapped book must return to the library" }
        Bridge.emit("play", "value" to false)
        eventually("Pause before button gestures") { !call().getBoolean("playing") }
        tap("Started"); tap("All")
        val menuLabel = "Book actions: $playableTitle"
        val playLabel = elements().map { it.getString("label") }.first {
            it in listOf("Play: $playableTitle", "Continue: $playableTitle", "Play again: $playableTitle")
        }
        for (label in listOf(menuLabel, playLabel)) {
            dragBookButton(label, -150f)
            check(call().getInt("filter") == 1) { "Swipe from $label must change category" }
            tap("All")
            dragBookButton(label, -150f, cancel = true)
            check(call().getInt("filter") == 0) { "Cancelled button swipe changed category" }
            dragBookButton(label, -100f, returnToStart = true)
            check(call().getInt("filter") == 0) { "Returning to the button must cancel the swipe and click" }
            dragBookButton(label, -16f)
            check(call().getInt("filter") == 0) { "Short button drag changed category" }
            val scroll = call().getDouble("books_scroll_y")
            dragBookButton(label, 0f, -120f)
            check(call().getInt("filter") == 0 && call().getDouble("books_scroll_y") < scroll - 40) {
                "Vertical scrolling must start on $label"
            }
            tap("Started"); tap("All")
        }
        tap(menuLabel)
        check(call().getBoolean("menu")) { "Menu button tap must still open the menu" }
        back()
        tap(playLabel)
        eventually("Library play button") { call().getBoolean("playing") }
        check(call().getInt("page") == 0 && !call().getBoolean("menu"))
        tap("Pause: $playableTitle")
        eventually("Library pause button") { !call().getBoolean("playing") }
        note("menu/play button swipes, cancellation, return-to-start, short drags, vertical scrolling and normal taps")
        note("swipes over book cards and scrolled lists, vertical scrolling, cancelled/short gestures and book taps")
    } finally {
        fixtureKeys.forEach { key ->
            call("action", "name" to "remove-book", "arg" to key)
            eventually("Remove swipe fixture $key") {
                val books = call().getJSONArray("books")
                (0 until books.length()).none { books.getJSONObject(it).getString("key") == key }
            }
        }
        call("page", "value" to 0)
    }
}
