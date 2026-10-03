package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import java.io.File

@UnstableApi
internal fun UiSuite.stalePickerResults(title: String) {
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }
        .single { it.getString("title") == title }.getString("key")
    val image = File(instrumentation.targetContext.cacheDir, "stale-picker-cover.png")
    Bitmap.createBitmap(24, 32, Bitmap.Config.ARGB_8888).apply {
        eraseColor(0xff456789.toInt())
        image.outputStream().use { compress(Bitmap.CompressFormat.PNG, 100, it) }
        recycle()
    }
    fun delivered(marker: String) {
        // The notice follows the picker through the controller and UI mailbox.
        Bridge.emit("error", "message" to marker)
        eventually("Picker result delivered") {
            call().let { it.getString("notice") == marker || it.getString("source_error") == marker }
        }
    }
    call("action", "name" to "begin-edit", "arg" to book)
    val cover = call().getString("cover_path")
    Bridge.emit("picked", "kind" to "cover", "uri" to image.absolutePath, "name" to image.name, "owner" to "-1")
    delivered("stale cover result delivered")
    check(call().let { !it.getBoolean("cover_loading") && it.getString("cover_path") == cover }) {
        "A late cover result for a removed book changed the current editor: ${call()}"
    }
    val oldCoverOwner = call().getString("picker_owner")
    tap("Cancel")
    call("action", "name" to "begin-edit", "arg" to book)
    Bridge.emit("picked", "kind" to "cover", "uri" to image.absolutePath, "name" to image.name, "owner" to oldCoverOwner)
    delivered("reopened cover result delivered")
    check(call().let { !it.getBoolean("cover_loading") && it.getString("cover_path") == cover }) {
        "A cancelled picker changed the reopened editor for the same book: ${call()}"
    }
    tap("Cancel")
    call("page", "value" to 4)
    Thread.sleep(350)
    tap("Update…")
    val source = call().getString("source_path")
    Bridge.emit("picked", "kind" to "source", "uri" to "content://stale/tree/other", "name" to "Old folder", "owner" to "-1")
    delivered("stale source result delivered")
    check(call().getString("source_path") == source) { "A late source result changed a different source editor" }
    val oldSourceOwner = call().getString("picker_owner")
    tap("Cancel")
    tap("Update…")
    Bridge.emit("picked", "kind" to "source", "uri" to "content://stale/tree/other", "name" to "Old folder", "owner" to oldSourceOwner)
    delivered("reopened source result delivered")
    check(call().getString("source_path") == source) { "A cancelled picker changed the reopened editor for the same source" }
    tap("Cancel")
    call("feedback", "kind" to "dismiss")
    call("page", "value" to 0)
    note("late cover and source picker results cannot change a different or reopened editor")
}

@UnstableApi
internal fun UiSuite.recreate() {
    var previous: MainActivity? = null
    main { previous = Bridge.activity.get(); previous!!.recreate() }
    eventually("Activity recreated") {
        var ready = false
        main { ready = Bridge.activity.get()?.let { it !== previous && it.hasWindowFocus() } == true }
        ready && runCatching { call().getDouble("width") > 0 }.getOrDefault(false)
    }
}

@UnstableApi
internal fun UiSuite.whileDetached(action: () -> Unit) {
    var detached: MainActivity? = null
    main { detached = Bridge.activity.get(); detached!!.finish() }
    eventually("Activity detached") {
        var gone = false
        main { gone = detached!!.isDestroyed && Bridge.activity.get() == null }
        gone
    }
    action()
    PlaybackSuite(instrumentation).call("state")
    Thread.sleep(300)
    instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    eventually("Activity attached") { runCatching { call().getDouble("width") > 0 }.getOrDefault(false) }
}

@UnstableApi
internal fun UiSuite.detachedNativeCallbacks() {
    lateinit var helper: Class<*>
    fun findInput(view: android.view.View): android.view.View? {
        if (view.javaClass.simpleName == "SlintInputView") return view
        if (view is android.view.ViewGroup) for (i in 0 until view.childCount) {
            findInput(view.getChildAt(i))?.let { return it }
        }
        return null
    }
    main {
        val input = checkNotNull(findInput(Bridge.activity.get()!!.window.decorView))
        helper = input.javaClass.classLoader!!.loadClass("SlintAndroidJavaHelper")
    }
    whileDetached {
        main {
            // Android can deliver queued configuration/IME callbacks after
            // onDestroy, while the background audio service is still alive.
            helper.getMethod("setNightMode", Int::class.javaPrimitiveType).invoke(null, 0x20)
            helper.getMethod("setFontScale", Float::class.javaPrimitiveType).invoke(null, 1f)
            helper.getMethod("popupMenuAction", Int::class.javaPrimitiveType).invoke(null, 1)
        }
    }
    check(call().getDouble("width") > 0)
    note("late native configuration and IME callbacks while the Activity is destroyed")
}

@UnstableApi
internal fun UiSuite.recreatedSources() {
    call("page", "value" to 4)
    Thread.sleep(350)
    tap("Update…")
    check(call().getInt("overlay") == 5)
    val source = call().getString("source_key")
    val owner = call().getString("picker_owner")
    val folder = "content://io.github.mny315.carlitos.test.sources/tree/slow"
    Bridge.emit("picked", "kind" to "source", "uri" to folder, "name" to "Selected folder", "owner" to owner)
    eventually("Source folder selected") { call().getString("source_path") == folder }
    recreate()
    eventually("Sources page restored") { call().getInt("page") == 4 }
    check(call().getInt("overlay") == 5 && call().getString("source_key") == source && call().getString("source_path") == folder) {
        "Recreation discarded the selected source folder: ${call()}"
    }
    val queued = "content://io.github.mny315.carlitos.test.sources/tree/root"
    whileDetached {
        Bridge.emit("picked", "kind" to "source", "uri" to queued, "name" to "Queued folder", "owner" to owner)
        repeat(64) { Bridge.emit("hidden", "value" to true) }
    }
    eventually("Queued source folder applied") {
        call().let { it.getInt("overlay") == 5 && it.getString("source_key") == source && it.getString("source_path") == queued }
    }
    tap("Cancel")
    call("page", "value" to 0)
    note("source folder changes and queued picker results survive Activity recreation")
}
@UnstableApi
internal fun UiSuite.recreatedEdits(title: String) {
    eventually("Recreation fixture") { call().getJSONArray("books").length() > 0 }
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }
        .single { it.getString("title") == title }.getString("key")
    val image = File(instrumentation.targetContext.cacheDir, "recreation-cover.png")
    Bitmap.createBitmap(32, 48, Bitmap.Config.ARGB_8888).apply {
        eraseColor(0xff56789a.toInt())
        image.outputStream().use { compress(Bitmap.CompressFormat.PNG, 100, it) }
        recycle()
    }
    call("action", "name" to "open-book", "arg" to book)
    Thread.sleep(350)
    tap("Edit book")
    check(call().getInt("overlay") == 2)
    call("action", "name" to "preview-cover", "arg" to image.absolutePath)
    eventually("Prepared custom cover") { !call().getBoolean("cover_loading") && call().getString("cover_path").isNotEmpty() }
    tap("Done")
    eventually("Cover saved") { call().getInt("overlay") == 0 }
    tap("Edit book"); tap("From audio files")
    check(call().getString("cover_path").isEmpty())
    recreate()
    eventually("Cover editor restored") { call().getInt("overlay") == 2 && !call().getBoolean("cover_loading") }
    check(call().getString("cover_path").isEmpty()) { "Recreation restored the custom cover that the user had removed: ${call()}" }
    check(call().getString("edit_key") == book)
    tap("Done")
    eventually("Removed cover saved") { call().getInt("overlay") == 0 }
    val saved = PlaybackSuite(instrumentation).call("state").getJSONArray("books")
    check((0 until saved.length()).map { saved.getJSONObject(it) }.single { it.getLong("id").toString() == book }.isNull("cover"))
    tap("Edit book")
    // The real picker delivers the same event while the native UI is absent.
    val owner = call().getString("picker_owner")
    whileDetached {
        Bridge.emit("picked", "kind" to "cover", "uri" to image.absolutePath, "name" to image.name, "owner" to owner)
        repeat(64) { Bridge.emit("hidden", "value" to true) }
    }
    eventually("Queued cover selection restored") {
        call().let { it.getInt("overlay") == 2 && !it.getBoolean("cover_loading") && it.getString("cover_path").isNotEmpty() }
    }
    tap("From audio files")
    val authority = "io.github.mny315.carlitos.test.playback"
    instrumentation.targetContext.contentResolver.call(android.net.Uri.parse("content://$authority"), "mode", "cover",
        Bundle().apply { putString("value", "slow") })
    val uri = android.provider.DocumentsContract.buildDocumentUri(authority, "cover")
    call("action", "name" to "preview-cover", "arg" to uri.toString())
    check(call().getBoolean("cover_loading"))
    recreate()
    eventually("Pending cover editor restored") { call().getInt("overlay") == 2 }
    eventually("Pending cover worker completed in recreated editor") {
        call().let { !it.getBoolean("cover_loading") && it.getString("cover_path").isNotEmpty() }
    }
    tap("From audio files")
    instrumentation.targetContext.contentResolver.call(android.net.Uri.parse("content://$authority"), "mode", "cover",
        Bundle().apply { putString("value", "slow") })
    call("action", "name" to "preview-cover", "arg" to uri.toString())
    check(call().getBoolean("cover_loading"))
    whileDetached {
        Thread.sleep(5500)
        repeat(64) { Bridge.emit("hidden", "value" to true) }
    }
    eventually("Completed cover survives background event burst") {
        call().let { it.getInt("overlay") == 2 && !it.getBoolean("cover_loading") && it.getString("cover_path").isNotEmpty() }
    }
    check(call().getString("edit_key") == book)
    tap("Cancel")
    call("page", "value" to 2)
    tap("Treat this folder as one book", "Checkbox")
    check(call().getBoolean("single_book"))
    recreate()
    eventually("Import page restored") { call().getInt("page") == 2 }
    check(call().getBoolean("single_book")) { "Recreation changed the import grouping mode" }
    tap("Treat this folder as one book", "Checkbox")
    grantFixtures(instrumentation, ui = true)
    val folder = android.provider.DocumentsContract.buildTreeDocumentUri("io.github.mny315.carlitos.test.sources", "slow")
    Bridge.emit("picked", "kind" to "folder", "uri" to folder.toString(), "name" to "Recreation fixture")
    call("action", "name" to "scan", "arg" to folder.toString())
    eventually("Slow scan started") { call().getBoolean("scanning") }
    recreate()
    eventually("Scanning page restored") { call().getInt("page") == 2 }
    check(call().getBoolean("scanning")) { "Recreation lost the running scan: ${call()}" }
    element("Cancel")
    eventually("Scan completed in recreated Activity") { call().getInt("drafts") == 1 && !call().getBoolean("scanning") }
    tap("Review parts")
    val draftTitle = call().getString("draft_title")
    recreate()
    eventually("Draft restored") { call().getInt("selected_draft") == 0 }
    check(call().getString("draft_title") == draftTitle && call().getInt("draft_files") == 1)
    whileDetached { PlaybackSuite(instrumentation).call("cancel_scan") }
    eventually("Cancelled draft list restored") { call().getInt("page") == 2 && call().getInt("drafts") == 0 }
    check(call().getInt("selected_draft") == -1) { "Recreation reopened a draft that no longer exists: ${call()}" }
    call("action", "name" to "scan", "arg" to folder.toString())
    eventually("Rescan completed") { call().getInt("drafts") == 1 && !call().getBoolean("scanning") }
    tap("Review parts")
    whileDetached { PlaybackSuite(instrumentation).call("import") }
    eventually("Background import returned to library") { call().getInt("page") == 0 && call().getInt("selected_draft") == -1 }
    call("page", "value" to 4)
    Thread.sleep(350)
    tap("Update…")
    val source = call().getString("source_key")
    check(call().getString("source_path").contains("/slow"))
    tap("Update")
    eventually("Source update started") { call().getBoolean("source_busy") }
    recreate()
    eventually("Source update dialog restored") { call().getInt("overlay") == 5 }
    check(call().getBoolean("source_busy") && call().getString("source_key") == source)
    element("Close")
    eventually("Source update completed") { !call().getBoolean("source_busy") && call().getInt("overlay") == 0 }
    val updated = PlaybackSuite(instrumentation).call("state").getJSONArray("books")
    val imported = (0 until updated.length()).map { updated.getJSONObject(it) }.single { it.getLong("source_id").toString() == source }
    call("action", "name" to "remove-book", "arg" to imported.getLong("id").toString())
    call("page", "value" to 0)
    note("removed cover, queued picker result, pending cover worker and import grouping survive Activity recreation")
    note("picker and completed cover results survive background event bursts without an Activity")
    note("running scan, cancellation control and draft editor survive Activity recreation")
    note("cancelled drafts stay closed; completed background imports and source updates restore their final state")
}

@UnstableApi
internal fun UiSuite.recreatedRemovedBook() {
    val playback = PlaybackSuite(instrumentation)
    val title = "Removed while Activity was detached"
    playback.fixture(title, listOf("removed-recreation"))
    eventually("Removal fixture visible") {
        val books = call().getJSONArray("books")
        (0 until books.length()).any { books.getJSONObject(it).getString("title") == title }
    }
    val books = call().getJSONArray("books")
    val book = (0 until books.length()).map { books.getJSONObject(it) }
        .single { it.getString("title") == title }.getString("key")
    call("action", "name" to "open-book", "arg" to book)
    check(call().getInt("page") == 1 && call().getString("selected") == book)
    whileDetached { playback.call("remove_book", "id" to book.toLong()) }
    eventually("Removed book returns to the library after recreation") { call().getInt("page") == 0 }
    check(call().getString("selected").isEmpty() && call().getInt("parts") == 0)
    note("a book removed while the Activity is absent cannot restore an empty book page")
}
