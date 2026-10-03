package io.github.mny315.carlitos

import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Synthetic documents in the separate test APK, independent of application data. */
class SourceFixtureProvider : DocumentsProvider() {
    private val app get() = requireNotNull(context)
    private val defaultColumns = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS)

    override fun onCreate() = true
    override fun queryRoots(projection: Array<out String>?): Cursor =
        MatrixCursor(projection ?: arrayOf(Root.COLUMN_ROOT_ID))

    private fun rows(projection: Array<out String>?, vararg ids: String): Cursor {
        val columns = if (ids.contentEquals(arrayOf("missing"))) arrayOf(Document.COLUMN_FLAGS,
            Document.COLUMN_DOCUMENT_ID, Document.COLUMN_MIME_TYPE, Document.COLUMN_DISPLAY_NAME)
            else projection ?: defaultColumns
        return MatrixCursor(columns).apply {
            ids.forEach { id ->
                addRow(columns.map { column ->
                    when (column) {
                        Document.COLUMN_DOCUMENT_ID -> id
                        Document.COLUMN_DISPLAY_NAME -> when {
                            id.startsWith("duplicate") -> "same.wav"
                            id == "audio" -> "Глава + 1.wav"
                            else -> id
                        }
                        Document.COLUMN_MIME_TYPE -> if (id in listOf("root", "duplicates", "slow")) Document.MIME_TYPE_DIR else "audio/wav"
                        Document.COLUMN_FLAGS -> 0
                        else -> null
                    }
                })
            }
        }
    }

    override fun queryDocument(id: String, projection: Array<out String>?): Cursor {
        if (id.startsWith("s4:")) return fixtureRows(projection, id)
        if (id == "denied") throw SecurityException("Test access revoked")
        return rows(projection, id)
    }

    override fun queryChildDocuments(parent: String, projection: Array<out String>?, sort: String?): Cursor {
        if (parent == "slow") {
            try { Thread.sleep(5000) }
            catch (error: InterruptedException) {
                Thread.currentThread().interrupt()
                throw IllegalStateException(error)
            }
            return rows(projection, "audio")
        }
        if (parent == "s4:bulk") return fixtureRows(projection, *Array(437) { "$parent/${437 - it}.mp3" })
        if (parent.startsWith("s4:")) {
            try {
                val children = app.assets.list(assetPath(parent)).orEmpty().map { "$parent/$it" }
                return fixtureRows(projection, *children.toTypedArray())
            } catch (error: IOException) { throw IllegalStateException(error) }
        }
        return if (parent == "duplicates") rows(projection, "duplicate1", "duplicate2") else rows(projection, "audio", "pipe")
    }

    override fun isChildDocument(parent: String, child: String) = true

    override fun openDocument(id: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
        if (id == "denied") throw SecurityException("Test access revoked")
        if (id.startsWith("stall-")) {
            val cancelled = CountDownLatch(1)
            signal?.setOnCancelListener { cancelled.countDown() }
            try {
                // Bound a failing regression too: the old client never cancels.
                cancelled.await(20, TimeUnit.SECONDS)
                signal?.throwIfCanceled()
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
                throw FileNotFoundException("Fixture interrupted")
            } finally { signal?.setOnCancelListener(null) }
        }
        try {
            if (id.startsWith("s4:")) return ParcelFileDescriptor.open(fixtureFile(id), ParcelFileDescriptor.MODE_READ_ONLY)
            if (id == "pipe") {
                val pipe = ParcelFileDescriptor.createPipe()
                pipe[1].close()
                return pipe[0]
            }
            val file = File(app.cacheDir, "source-fixture.wav")
            if (!file.exists()) {
                val buffer = ByteBuffer.allocate(44 + 16000).order(ByteOrder.LITTLE_ENDIAN)
                buffer.put("RIFF".toByteArray()).putInt(36 + 16000).put("WAVEfmt ".toByteArray())
                    .putInt(16).putShort(1).putShort(1).putInt(8000).putInt(16000)
                    .putShort(2).putShort(16).put("data".toByteArray()).putInt(16000)
                file.writeBytes(buffer.array())
            }
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        } catch (error: IOException) { throw FileNotFoundException(error.toString()) }
    }

    private fun assetPath(id: String): String {
        require(!id.contains("..")) { "Invalid fixture ID" }
        if (id == "s4:bulk") return "Carlitos-import/Автор/Книга/CD 1"
        if (id.startsWith("s4:bulk/")) return "Carlitos-import/Автор/Книга/CD 1/10.mp3"
        return "Carlitos-import" + id.substringAfter('/', "").let { if (it.isEmpty()) "" else "/$it" }
    }

    private fun fixtureFile(documentId: String): File {
        // Distinct document IDs share one immutable synthetic recording; the
        // production scanner still queries and opens all 437 provider documents.
        val id = if (documentId.startsWith("s4:bulk/")) "s4:old/Автор/Книга/CD 1/10.mp3" else documentId
        val file = File(app.cacheDir, "import/${id.substring(3)}")
        if (!file.exists()) {
            file.parentFile?.mkdirs()
            app.assets.open(assetPath(id)).use { input ->
                file.outputStream().use { output ->
                    if (id == "s4:bad/Автор/Книга/CD 2/1.mp3") output.write(byteArrayOf(1, 2, 3))
                    else input.copyTo(output)
                }
            }
        }
        return file
    }

    private fun fixtureRows(projection: Array<out String>?, vararg ids: String): Cursor {
        val columns = projection ?: defaultColumns
        val cursor = MatrixCursor(columns)
        try {
            ids.forEach { id ->
                val directory = !app.assets.list(assetPath(id)).isNullOrEmpty()
                cursor.addRow(columns.map { column ->
                    when (column) {
                        Document.COLUMN_DOCUMENT_ID -> id
                        Document.COLUMN_DISPLAY_NAME -> id.substringAfterLast('/')
                        Document.COLUMN_MIME_TYPE -> if (directory) Document.MIME_TYPE_DIR else "application/octet-stream"
                        Document.COLUMN_SIZE -> if (directory) null else fixtureFile(id).length()
                        Document.COLUMN_LAST_MODIFIED -> 1700000000000L
                        Document.COLUMN_FLAGS -> 0
                        else -> null
                    }
                })
            }
        } catch (error: IOException) { throw IllegalStateException(error) }
        return cursor
    }
}
