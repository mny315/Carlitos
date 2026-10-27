package io.github.mny315.carlitos

import androidx.media3.common.util.UnstableApi
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

@UnstableApi
internal fun UiSuite.sourceSelectionChecks() {
    fun fixture(root: File, label: String): File {
        root.mkdirs()
        val file = File(root, "part.wav").apply { writeText("Original audio $label") }
        val media = JSONObject().put("id", 0).put("source_id", 0).put("uri", file.toURI().toString())
            .put("relative", file.name).put("identity", "selection:$label").put("size", file.length())
            .put("modified", 1).put("duration", 60000).put("title", "Part").put("artist", "Device suite")
            .put("album", "Source $label").put("track", JSONObject.NULL).put("disc", JSONObject.NULL)
            .put("cover", JSONObject.NULL).put("sort_tags_read", true).put("chapters", JSONArray())
        PlaybackSuite(instrumentation).call("fixture", "root" to root.toURI().toString(),
            "title" to "Selection $label", "files" to JSONArray().put(media), "folder" to true)
        return file
    }
    val files = (1..3).map { index ->
        fixture(File(instrumentation.targetContext.filesDir, "selection/Source $index"), index.toString())
    }
    call("page", "value" to 4)
    fun sources(): List<JSONObject> {
        val rows = call().getJSONArray("sources")
        return (0 until rows.length()).map { rows.getJSONObject(it) }
    }
    eventually("Three source fixtures") { files.all { f -> sources().any { it.getString("path").trimEnd('/') == f.parent } } }
    val before = sources().size
    val selected = sources().filter { row -> files.take(2).any { it.parent == row.getString("path").trimEnd('/') } }
    fun selectTwo() {
        for (row in selected) tap("Select folder: ${File(row.getString("path")).name}", "Checkbox")
        check(call().getInt("selected_source_count") == 2)
    }
    tap("Select folders")
    check(call().getBoolean("source_selection") && call().getInt("selected_source_count") == 0)
    tap("Select all", "Checkbox")
    check(call().getInt("selected_source_count") == before)
    tap("Select all", "Checkbox")
    check(call().getInt("selected_source_count") == 0)
    selectTwo()
    capture("sources-selection")
    recreate()
    eventually("Source selection restored") { call().getBoolean("source_selection") && call().getInt("selected_source_count") == 2 }
    check(sources().filter { it.getBoolean("selected") }.map { it.getString("key") }.toSet() == selected.map { it.getString("key") }.toSet())
    lateinit var added: File
    whileDetached { added = fixture(File(files[0].parentFile, "New member"), "new-member") }
    val retained = selected.single { it.getString("path").trimEnd('/') == files[1].parent }
    eventually("Changed source selection restored safely") { call().getInt("selected_source_count") == 1 }
    check(sources().filter { it.getBoolean("selected") }.map { it.getString("key") }.toSet() == setOf(retained.getString("key")))
    call("action", "name" to "deselect-sources")
    selectTwo()
    tap("Remove selected folders")
    check(call().getInt("overlay") == 1 && call().getInt("remove_source_count") == 2)
    capture("sources-selection-confirmation")
    tap("Cancel")
    check(call().getInt("selected_source_count") == 2 && sources().size == before)
    tap("Remove selected folders")
    back()
    check(call().getBoolean("source_selection") && call().getInt("overlay") == 0)
    back()
    check(call().getInt("page") == 4 && !call().getBoolean("source_selection"))
    tap("Select folders")
    selectTwo()
    tap("Remove selected folders")
    tap("Remove")
    eventually("Only selected sources removed") { sources().size == before - 2 && !call().getBoolean("source_selection") }
    check(sources().none { row -> selected.any { it.getString("key") == row.getString("key") } })
    check(sources().any { it.getString("path").trimEnd('/') == files.last().parent })
    check(added.exists()) { "Removing a source must retain newly discovered audio too" }
    tap("Select folders")
    tap("Select all", "Checkbox")
    tap("Remove selected folders")
    tap("Remove")
    eventually("All sources removed") { sources().isEmpty() && !call().getBoolean("source_selection") }
    files.forEachIndexed { index, file -> check(file.readText() == "Original audio ${index + 1}") { "Source removal must never delete audio files" } }
    capture("sources-empty")
    note("source multi-selection, select all, recreation, cancellation, Back, batch deletion and original files retained")
}
