package io.github.mny315.carlitos

import android.view.View
import android.view.ViewGroup
import android.view.KeyEvent
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun UiSuite.imeSelectionAndComposition(title: String) {
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }
        .single { it.getString("title") == title }.getString("key")
    call("action", "name" to "begin-edit", "arg" to book)
    tap("Title", "TextInput")
    eventually("IME editor focused") { call().getBoolean("keyboard") }
    fun input(view: View): View? {
        if (view.javaClass.simpleName == "SlintInputView") return view
        if (view is ViewGroup) for (index in 0 until view.childCount) {
            input(view.getChildAt(index))?.let { return it }
        }
        return null
    }
    lateinit var connection: InputConnection
    main {
        connection = checkNotNull(input(Bridge.activity.get()!!.window.decorView))
            .onCreateInputConnection(EditorInfo())!!
        check(connection.setSelection(0, title.length))
    }
    // Hardware input is handled by Slint; an IME-only selection must reach
    // that same editor before the next key replaces its selected text.
    Thread.sleep(250)
    instrumentation.sendStringSync("x")
    eventually("IME selection reaches the native editor") { call().getString("title") == "x" }
    main {
        check(connection.setSelection(0, 1))
        check(connection.setComposingText("Слово", 1))
    }
    Thread.sleep(250)
    main { check(connection.finishComposingText()) }
    eventually("Finishing composition commits the title") { call().getString("title") == "Слово" }
    main {
        connection.beginBatchEdit()
        check(connection.setSelection(0, 5))
        check(connection.commitText("Закрытие", 1))
        connection.closeConnection()
    }
    eventually("Closing the input connection flushes its unfinished batch") {
        call().getString("title") == "Закрытие"
    }
    main {
        check(!connection.setSelection(0, 0)) { "A closed connection must not change the new editor" }
        connection = checkNotNull(input(Bridge.activity.get()!!.window.decorView))
            .onCreateInputConnection(EditorInfo())!!
        check(connection.beginBatchEdit())
        check(connection.beginBatchEdit())
        check(connection.setSelection(0, 8))
        check(connection.commitText("Пакет", 1))
    }
    Thread.sleep(250)
    check(call().getString("title") == "Закрытие") { "An unfinished batch leaked a partial edit" }
    main { check(connection.endBatchEdit()) }
    Thread.sleep(250)
    check(call().getString("title") == "Закрытие") { "An inner batch committed the outer batch" }
    main { check(!connection.endBatchEdit()) }
    eventually("The outer batch commits the new title") { call().getString("title") == "Пакет" }
    main {
        check(!connection.endBatchEdit())
        check(connection.setSelection(0, 5))
        check(connection.commitText("Готово", 1))
    }
    eventually("An extra batch end cannot break later input") { call().getString("title") == "Готово" }
    fun shrinkSelection(anchor: Int, cursor: Int, arrow: Int, expected: String) {
        main { check(connection.setSelection(anchor, cursor)) }
        Thread.sleep(250)
        instrumentation.sendKeySync(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_SHIFT_LEFT))
        try {
            instrumentation.sendKeyDownUpSync(arrow)
        } finally {
            instrumentation.sendKeySync(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_SHIFT_LEFT))
        }
        instrumentation.sendStringSync("x")
        eventually("Shift-arrow shrinks the IME selection from its moving end ($anchor, $cursor)") {
            call().getString("title") == expected
        }
        main {
            check(connection.setSelection(0, expected.length))
            check(connection.commitText("Готово", 1))
        }
        eventually("The title is restored after extending selection") { call().getString("title") == "Готово" }
    }
    shrinkSelection(2, 5, KeyEvent.KEYCODE_DPAD_LEFT, "Гоxво")
    shrinkSelection(5, 2, KeyEvent.KEYCODE_DPAD_RIGHT, "Готxо")
    main {
        check(connection.setSelection(3, 3))
        check(connection.setComposingRegion(1, 4))
    }
    eventually("An IME composition span reaches Slint without replacing text") { call().getString("title") == "Гво" }
    Thread.sleep(250)
    main {
        val before = connection.getTextBeforeCursor(20, 0).toString()
        check(before == "Гот") { "Composition feedback moved the IME caret: $before" }
        check(connection.finishComposingText())
    }
    eventually("The existing composition is committed") { call().getString("title") == "Готово" }
    main { check(connection.setSelection(6, 6)) }
    main { check(connection.setComposingRegion(1, 4)) }
    Thread.sleep(250)
    check(call().getString("title") == "Готово") { "A composition outside the caret reordered the text" }
    main { check(connection.finishComposingText()) }
    eventually("A composition outside the caret retains the whole word") { call().getString("title") == "Готово" }
    val recreatedTitle = "Пересоздание 📚"
    main {
        check(connection.setSelection(0, 6))
        check(connection.setComposingText(recreatedTitle, 1))
    }
    eventually("The draft is still being composed before recreation") { call().getString("title").isEmpty() }
    recreate()
    eventually("Pending keyboard composition survives Activity recreation") {
        call().getInt("overlay") == 2 && call().getString("title") == recreatedTitle
    }
    tap("Title", "TextInput")
    main {
        // A late IME callback can retain the destroyed Activity's connection.
        // It must never replace the text in the newly focused editor.
        connection.commitText("STALE", 1)
        connection.finishComposingText()
    }
    Thread.sleep(250)
    check(call().getString("title") == recreatedTitle) { "An old Activity's IME connection overwrote the restored title" }
    main {
        connection = checkNotNull(input(Bridge.activity.get()!!.window.decorView))
            .onCreateInputConnection(EditorInfo())!!
    }
    val composedTitle = "$title — IME"
    main {
        check(connection.setSelection(0, recreatedTitle.length))
        check(connection.setComposingText(composedTitle, 1))
    }
    eventually("The new title is still being composed") { call().getString("title").isEmpty() }
    capture("ime-title-composition")
    tap("Done")
    eventually("Saving commits a title that is still being composed") { call().getInt("overlay") == 0 }
    call("action", "name" to "begin-edit", "arg" to book)
    check(call().getString("title") == composedTitle) { "Saving lost the pending keyboard composition" }
    tap("Title", "TextInput")
    main {
        connection = checkNotNull(input(Bridge.activity.get()!!.window.decorView))
            .onCreateInputConnection(EditorInfo())!!
        check(connection.setSelection(0, composedTitle.length))
        check(connection.commitText(title, 1))
    }
    eventually("The fixture title is restored") { call().getString("title") == title }
    tap("Done")
    eventually("The fixture title is saved") { call().getInt("overlay") == 0 }
    call("page", "value" to 0)
    note("IME selections, composition spans, nested batches, closing connections and pending text survive saving and Activity recreation")
}
