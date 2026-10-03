package io.github.mny315.carlitos

import android.content.ContentProvider
import android.content.ContentValues
import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException

/** General provider slices: DocumentsProvider.openAssetFile is final. */
class SliceFixtureProvider : ContentProvider() {
    override fun onCreate() = true
    override fun getType(uri: Uri): String = when (DocumentsContract.getDocumentId(uri)) {
        "slice-wav" -> "audio/wav"
        "slice-m4b" -> "audio/mp4"
        else -> "audio/mpeg"
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor {
        val id = DocumentsContract.getDocumentId(uri)
        val columns = projection ?: arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME)
        return MatrixCursor(columns).apply {
            addRow(columns.map { column ->
                when (column) {
                    Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME -> id
                    Document.COLUMN_MIME_TYPE -> getType(uri)
                    Document.COLUMN_FLAGS -> 0
                    else -> null
                }
            })
        }
    }

    override fun openAssetFile(uri: Uri, mode: String): AssetFileDescriptor {
        val id = DocumentsContract.getDocumentId(uri)
        val path = when (id) {
            "slice-wav" -> "Carlitos-playback/tone.wav"
            "slice-mp3" -> "Carlitos-import/Автор/Книга/CD 1/10.mp3"
            "slice-m4b" -> "Carlitos-import/Форматы/QuickTime.m4b"
            else -> throw FileNotFoundException(id)
        }
        try {
            val app = requireNotNull(context)
            val bytes = app.assets.open(path).use { it.readBytes() }
            val file = File(app.cacheDir, id)
            file.outputStream().use { output ->
                output.write(ByteArray(137))
                output.write(bytes)
                output.write(ByteArray(251))
            }
            return AssetFileDescriptor(ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY), 137, bytes.size.toLong())
        } catch (error: IOException) { throw FileNotFoundException(error.toString()) }
    }
}
