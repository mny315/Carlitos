package io.github.mny315.carlitos

import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.net.Uri
import android.os.CancellationSignal
import android.provider.DocumentsContract as DC
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Deadlines cover provider queries and descriptor acquisition. Cancellation is
 * cooperative: the provider must honor the signal while its call is blocked. */
internal object DocumentIo {
    private val resolver get() = Bridge.context.contentResolver
    private val timer = ScheduledThreadPoolExecutor(1) {
        Thread(it, "carlitos-provider-timeout").apply { isDaemon = true }
    }.apply { removeOnCancelPolicy = true }
    private val columns = arrayOf(DC.Document.COLUMN_DOCUMENT_ID, DC.Document.COLUMN_DISPLAY_NAME,
        DC.Document.COLUMN_MIME_TYPE, DC.Document.COLUMN_SIZE, DC.Document.COLUMN_LAST_MODIFIED,
        DC.Document.COLUMN_FLAGS)
    private fun <T> timed(request: (CancellationSignal) -> T): T {
        val signal = CancellationSignal()
        val timeout = timer.schedule({ signal.cancel() }, 15, TimeUnit.SECONDS)
        try { return request(signal) }
        finally { timeout.cancel(false) }
    }
    fun <T> read(uri: Uri, consume: (AssetFileDescriptor) -> T): T {
        // ContentResolver routes read-only opens through openTypedAssetFile.
        // DocumentsProvider's default implementation then discards the signal.
        // Call openAssetFile directly, retaining slice offsets and the provider
        // reference throughout metadata extraction/descriptor duplication.
        val provider = resolver.acquireContentProviderClient(uri) ?: error("Provider unavailable")
        return provider.use {
            val asset = timed { signal ->
                val opened = provider.openAssetFile(uri, "r", signal) ?: error("Cannot open document")
                // A provider can return an FD despite cancellation. Ownership
                // is not transferred on failure, so close that FD here.
                if (signal.isCanceled) {
                    opened.close()
                    signal.throwIfCanceled()
                }
                opened
            }
            asset.use(consume)
        }
    }
    fun query(uri: Uri, parent: Uri): JSONArray = timed { signal ->
        val result = JSONArray()
        (resolver.query(uri, columns, null, null, null, signal) ?: error("Provider unavailable")).use { cursor ->
            val indices = IntArray(columns.size) { cursor.getColumnIndex(columns[it]) }
            while (true) {
                signal.throwIfCanceled()
                if (!cursor.moveToNext()) break
                result.put(row(cursor, indices, parent))
            }
            signal.throwIfCanceled()
        }
        result
    }
    private fun row(cursor: Cursor, indices: IntArray, parent: Uri): JSONObject {
        fun text(index: Int): String {
            val column = indices[index]
            return if (column < 0 || cursor.isNull(column)) "" else cursor.getString(column)
        }
        fun number(index: Int): Any {
            val column = indices[index]
            return if (column < 0 || cursor.isNull(column) || cursor.getLong(column) < 0) JSONObject.NULL else cursor.getLong(column)
        }
        val id = text(0)
        check(id.isNotEmpty()) { "Provider returned no document ID" }
        val uri = if (DC.isTreeUri(parent)) DC.buildDocumentUriUsingTree(parent, id)
            else DC.buildDocumentUri(parent.authority!!, id)
        return JSONObject().put("uri", uri.toString()).put("name", text(1))
            .put("directory", text(2) == DC.Document.MIME_TYPE_DIR)
            .put("mime", text(2)).put("size", number(3)).put("modified", number(4))
            .put("virtual", (number(5) as? Long ?: 0L).toInt() and DC.Document.FLAG_VIRTUAL_DOCUMENT != 0)
    }
}
