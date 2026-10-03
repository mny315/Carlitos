package io.github.mny315.carlitos

import android.content.Intent
import android.content.pm.ActivityInfo
import androidx.media3.common.util.UnstableApi
import java.io.File

@UnstableApi
internal fun UiSuite.presentationChecks(title: String, packageName: String) {
    if (call().getBoolean("keyboard")) { back(); eventually("Initial keyboard hidden") { !call().getBoolean("keyboard") } }
    eventually("Library fixture") { call().getJSONArray("books").length() > 0 }
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }.first { it.getString("title") == title }.getString("key")
    call("action", "name" to "open-book", "arg" to book)
    Thread.sleep(500)
    check(call().getInt("parts") >= 4)
    check(elements().any { it.getString("label") == "First" }) { "Chapters are not directly visible" }
    check(elements().none { it.getString("label") == "Contents" })
    capture("book-portrait")
    note("chapters visible directly, long title, portrait touch controls")
    tap("Volume")
    check(call().getInt("overlay") == 7)
    val volumeSlider = element("Volume", "Slider")
    check(volumeSlider.getDouble("w") >= 250)
    check(volumeSlider.getDouble("y") > call().getDouble("height") / 2) { "Volume must open at the bottom" }
    check(element("Done").getDouble("y") > call().getDouble("height") * .8) { "Done must stay near the bottom" }
    tapAt(volumeSlider.getDouble("x") - 10, volumeSlider.getDouble("y") + volumeSlider.getDouble("h") / 2)
    check(call().getInt("overlay") == 7) { "Panel padding must not dismiss the sheet" }
    drag("Volume", .2f, .65f)
    eventually("Volume drag") { kotlin.math.abs(call().getDouble("volume") - .65) < .04 }
    tap("Mute")
    eventually("Mute updates UI") { call().getBoolean("muted") }
    recreate()
    eventually("Volume sheet restored") { call().getInt("overlay") == 7 }
    check(call().getBoolean("muted") && kotlin.math.abs(call().getDouble("volume") - .65) < .04)
    tap("Unmute")
    eventually("Unmute updates UI") { !call().getBoolean("muted") }
    capture("volume")
    tap("Navigation menu")
    check(call().getInt("overlay") == 0 && call().getInt("page") == 1) { "Outside tap must close without opening navigation: ${call()}" }
    tap("Volume")
    check(kotlin.math.abs(call().getDouble("volume") - .65) < .04) { "Outside dismissal must keep volume" }
    back(); check(call().getInt("page") == 1 && call().getInt("overlay") == 0) { "Back after volume: ${call()}" }
    note("bottom volume sheet, inside/outside taps, retained volume, Back closes only dialog")
    tap("Playback speed")
    check(element("Playback speed", "Slider").getDouble("y") > call().getDouble("height") / 2) { "Speed must open at the bottom" }
    tap("1.5×")
    check(element("Playback speed", "Slider").getDouble("value") == 40.0) { "Speed slider must match the preset after volume was dragged" }
    capture("speed")
    drag("Playback speed", .3f, .5f)
    eventually("Speed drag") { kotlin.math.abs(call().getDouble("rate") - 1.75) < .001 }
    tap("1.25×")
    check(element("Playback speed", "Slider").getDouble("value") == 30.0) { "Preset must move the slider after dragging" }
    tap("Faster")
    check(element("Playback speed", "Slider").getDouble("value") == 32.0) { "Step button must move the slider" }
    drag("Playback speed", .3f, .5f)
    eventually("Speed drag after preset") { kotlin.math.abs(call().getDouble("rate") - 1.75) < .001 }
    tap("Done")
    check(call().getInt("overlay") == 0)
    tap("Playback speed")
    check(kotlin.math.abs(call().getDouble("rate") - 1.75) < .001) { "Done must keep speed" }
    tap("Navigation menu")
    check(call().getInt("overlay") == 0 && call().getInt("page") == 1)
    tap("Playback speed")
    check(kotlin.math.abs(call().getDouble("rate") - 1.75) < .001) { "Outside dismissal must keep speed" }
    back(); check(call().getInt("page") == 1)
    note("bottom speed sheet, presets and slider, Done and outside tap retain the same value")
    tap("Play: First")
    eventually("Chapter touch plays") { call().getBoolean("playing") }
    drag("Position in part", .15f, .55f)
    eventually("Seek drag") { call().getDouble("position") in .50.. .65 }
    tap("Pause")
    note("chapter playback and touch seeking")
    back(); check(call().getInt("page") == 0)
    tap("Book actions: $title")
    check(call().getBoolean("menu"))
    back(); check(!call().getBoolean("menu") && call().getInt("page") == 0)
    tap("Sort by: Started first")
    tap("Title")
    check(call().getInt("overlay") == 0)
    capture("library-sort")
    tap("Search books")
    eventually("Search keyboard") { call().getBoolean("keyboard") }
    instrumentation.sendStringSync("android ui")
    eventually("Search input") { call().getString("query") == "android ui" }
    capture("search-keyboard")
    back(); eventually("Back hides keyboard") { !call().getBoolean("keyboard") }
    check(call().getString("query") == "android ui") { "Back must preserve search after hiding IME: ${call()}" }
    tap("All")
    swipeBooks(listOf(.75f to .8f, .25f to .8f), captureDrag = true)
    check(call().getInt("filter") == 1)
    swipeBooks(listOf(.75f to .8f, .25f to .8f))
    check(call().getInt("filter") == 2 && call().getJSONArray("books").length() == 0)
    capture("library-swipe-empty")
    swipeBooks(listOf(.75f to .8f, .25f to .8f))
    check(call().getInt("filter") == 2) { "Completed must not wrap to All" }
    swipeBooks(listOf(.25f to .8f, .75f to .8f))
    check(call().getInt("filter") == 1) { "Swipe must work with no matching books" }
    swipeBooks(listOf(.25f to .8f, .75f to .8f))
    check(call().getInt("filter") == 0)
    swipeBooks(listOf(.25f to .8f, .75f to .8f))
    check(call().getInt("filter") == 0) { "All must not wrap to Completed" }
    swipeBooks(listOf(.5f to .8f, .46f to .8f))
    check(call().getInt("filter") == 0) { "Small drags must not switch filters" }
    swipeBooks(listOf(.75f to .85f, .75f to .65f, .25f to .65f))
    check(call().getInt("filter") == 0) { "A vertical gesture must stay vertical" }
    swipeBooks(listOf(.75f to .8f, .25f to .8f), cancel = true)
    check(call().getInt("filter") == 0) { "Cancelled gestures must not switch filters" }
    check(call().getString("query") == "android ui") { "Swipes must preserve the search query" }
    tap("Completed"); check(call().getInt("filter") == 2)
    tap("All"); check(call().getInt("filter") == 0)
    note("blank-space swipes, empty results, boundaries, short/vertical/cancelled gestures, search and tab taps")
    back(); check(call().getString("query").isEmpty())
    element("Search books")
    note("sort choice, real keyboard text input, keyboard-first Back")
    call("action", "name" to "open-book", "arg" to book)
    Thread.sleep(350)
    tap("Edit book")
    tap("Title", "TextInput")
    eventually("Editor keyboard") { call().getBoolean("keyboard") }
    instrumentation.sendStringSync(" edited")
    capture("editor-keyboard")
    back(); eventually("Editor keyboard hidden") { !call().getBoolean("keyboard") }
    check(call().getInt("overlay") == 2)
    val edited = call().getString("title")
    var previous: MainActivity? = null
    main { previous = Bridge.activity.get(); previous!!.recreate() }
    eventually("New Activity attached") {
        var ready = false
        main { ready = Bridge.activity.get()?.let { it !== previous && it.hasWindowFocus() } == true }
        ready
    }
    eventually("Editor restored after recreation") { try { call().getInt("overlay") == 2 && call().getString("title") == edited } catch (_: Exception) { false } }
    capture("recreated-editor")
    back(); check(call().getInt("overlay") == 0)
    note("editor input and unsaved title survive Activity recreation")
    main { Bridge.activity.get()!!.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
    eventually("Landscape") { call().getDouble("width") > call().getDouble("height") }
    check(call().getInt("page") == 1)
    capture("book-landscape")
    tap("Playback controls"); check(call().getInt("overlay") == 8)
    capture("landscape-player")
    back()
    tap("Playback speed"); capture("landscape-speed"); tap("Done")
    check(call().getInt("overlay") == 0) { "Done must remain reachable in landscape" }
    main { Bridge.activity.get()!!.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
    eventually("Portrait") { call().getDouble("height") > call().getDouble("width") }
    shell("settings put system font_scale 2.0")
    eventually("System font scale") { call().getDouble("font_scale") > 1.99 }
    capture("book-large-font")
    tap("Edit book"); capture("editor-large-font"); back()
    back(); capture("library-large-font")
    val narrowWidth = (call().getDouble("scale") * 360).toInt()
    shell("wm size ${narrowWidth}x${narrowWidth * 20 / 9}")
    eventually("Narrow screen") { call().getDouble("width") <= 360.1 }
    capture("library-narrow-large-font")
    tap("Playback speed"); capture("speed-narrow-large-font"); tap("Done")
    tap("Volume"); capture("volume-narrow-large-font"); tap("Done")
    check(call().getInt("overlay") == 0) { "Done must remain reachable with large fonts" }
    note("rotation preserves page; full playback controls in landscape; system font 200 percent")
    tap("Navigation menu"); tap("Settings")
    tap("English"); tap("Русский")
    check(!call().getBoolean("english"))
    tap("Как в системе"); tap("Светлая тема")
    check(!call().getBoolean("dark"))
    main { @Suppress("DEPRECATION")
        check(Bridge.activity.get()!!.window.decorView.systemUiVisibility and android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR != 0)
    }
    capture("settings-russian-light")
    note("manual Russian language and light theme")
    tap("Светлая тема"); tap("Как в системе")
    shell("cmd uimode night no")
    eventually("System light theme") { !call().getBoolean("dark") }
    shell("cmd uimode night yes")
    eventually("System dark theme") { call().getBoolean("dark") }
    tap("Русский"); tap("Язык системы")
    shell("cmd locale set-app-locales $packageName --locales ru")
    eventually("System Russian locale") { !call().getBoolean("english") }
    shell("cmd locale set-app-locales $packageName --locales en")
    eventually("System English locale") { call().getBoolean("english") }
    note("live system theme and Android application locale; 360 dp at 200 percent font")
    call("page", "value" to 0)
    Bridge.emit("volume", "value" to .1)
    call("action", "name" to "resume", "arg" to book)
    val playback = PlaybackSuite(instrumentation)
    eventually("Play before background") { playback.call("state").getJSONObject("playback").getBoolean("playing") }
    val before = playback.call("state").getJSONObject("playback").getLong("position")
    back()
    eventually("Back backgrounds the library") {
        var hidden = false
        main { hidden = Bridge.activity.get()?.hasWindowFocus() == false }
        hidden
    }
    Thread.sleep(1000)
    val after = playback.call("state").getJSONObject("playback")
    check(after.getBoolean("playing") && after.getLong("position") > before)
    Bridge.emit("play", "value" to false)
    instrumentation.targetContext.startActivity(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    note("Back from library backgrounds Activity while real playback continues")
    File(output, "haptics.txt").writeText(shell("dumpsys vibrator_manager"))
}
