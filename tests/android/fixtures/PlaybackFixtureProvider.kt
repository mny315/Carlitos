package io.github.mny315.carlitos

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.graphics.Bitmap
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException

/** Synthetic audio and artwork only. This provider is never in the application APK. */
class PlaybackFixtureProvider : ContentProvider() {
    private val app get() = requireNotNull(context)
    private val preferences get() = app.getSharedPreferences("playback", Context.MODE_PRIVATE)

    private fun asset(id: String): String = when (id) {
        "tone", "part2", "fault" -> "tone.wav"
        "silence" -> "silence.wav"
        "pcm-16000", "pcm-44100", "pcm-96000" -> "$id.wav"
        "mp3-mono" -> "import-mono.mp3"
        "mp3-stereo" -> "import-stereo.mp3"
        "aac" -> "import.aac"
        "m4b" -> "import.m4b"
        "he-aac-v1", "he-aac-v2" -> "import-$id.m4b"
        "flac" -> "import.flac"
        "flac-large" -> "import-large-frame.flac"
        "cover" -> "cover.png"
        else -> throw FileNotFoundException(id)
    }

    override fun onCreate(): Boolean {
        // Reinstalling the test APK preserves its cache. Regenerated assets
        // must replace old fixture bytes on the next provider process start.
        app.cacheDir.listFiles { _, name -> name.startsWith("playback-") }?.forEach { it.delete() }
        return true
    }

    override fun getType(uri: Uri): String? = try {
        when (asset(uri.lastPathSegment ?: "").substringAfterLast('.')) {
            "png" -> "image/png"
            "mp3" -> "audio/mpeg"
            "aac" -> "audio/aac"
            "m4b" -> "audio/mp4"
            "flac" -> "audio/flac"
            else -> "audio/wav"
        }
    } catch (_: FileNotFoundException) { null }

    override fun insert(uri: Uri, values: ContentValues?): Uri = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()

    override fun call(method: String, arg: String?, extras: Bundle?): Bundle {
        if (method != "mode") throw UnsupportedOperationException()
        preferences.edit().putString(requireNotNull(arg), extras?.getString("value", "normal") ?: "normal").commit()
        return Bundle.EMPTY
    }

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor {
        val id = DocumentsContract.getDocumentId(uri)
        val result = MatrixCursor(arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
            Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS))
        try {
            val name = asset(id)
            val length = if (id == "cover") null else try {
                app.assets.openFd("Carlitos-playback/$name").use { it.length }
            } catch (_: IOException) { null } // Compressed assets have unknown length.
            result.addRow(arrayOf<Any?>(id, name, getType(uri), length, 1, 0))
        } catch (error: IOException) { throw IllegalStateException(error) }
        return result
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw FileNotFoundException("Read only")
        val id = DocumentsContract.getDocumentId(uri)
        val name = asset(id)
        val state = preferences.getString(id, "normal")
        if (state == "missing") throw FileNotFoundException("Fixture source unavailable")
        if (state == "slow") {
            // Delay one open only; Media3 may reopen the descriptor for a seek.
            preferences.edit().putString(id, "normal").commit()
            try { Thread.sleep(5000) }
            catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
                throw FileNotFoundException("Fixture load interrupted")
            }
        }
        try {
            val file = File(app.cacheDir, "playback-$id-$state")
            if (!file.exists()) file.outputStream().use { output ->
                when {
                    id == "cover" -> {
                        val bitmap = Bitmap.createBitmap(32, 48, Bitmap.Config.ARGB_8888)
                        try {
                            bitmap.eraseColor(0xffb15b34.toInt())
                            bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
                        } finally { bitmap.recycle() }
                    }
                    state == "broken" -> output.write(byteArrayOf(1, 2, 3, 4))
                    else -> app.assets.open("Carlitos-playback/$name").use { it.copyTo(output, 32768) }
                }
            }
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        } catch (error: IOException) { throw FileNotFoundException(error.toString()) }
    }
}
