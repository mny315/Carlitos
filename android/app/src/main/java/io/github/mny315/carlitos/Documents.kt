package io.github.mny315.carlitos

import android.content.Intent
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract as DC
import android.system.Os
import android.system.OsConstants
import org.json.JSONArray
import org.json.JSONObject

/** Provider I/O only. Rust owns scanning, grouping, identity and library state. */
object Documents {
    fun accessError(error: Throwable): String? {
        for (cause in generateSequence(error) { it.cause }.take(8)) {
            when (cause) {
                is SecurityException -> return "Access to this source was revoked. Select its folder again."
                is java.io.FileNotFoundException -> return "This document is unavailable. Check its source or select the folder again."
                is android.os.OperationCanceledException -> return "The source did not respond in time. Try again."
                is android.system.ErrnoException -> if (cause.errno == OsConstants.ESPIPE)
                    return "This document does not support seeking. Choose a local audio file."
            }
        }
        return null
    }
    private val resolver get() = Bridge.context.contentResolver
    fun document(uri: Uri): Uri {
        require(uri.scheme == "content" && uri.authority != null) { "Select a local document" }
        return if (DC.isTreeUri(uri) && !DC.isDocumentUri(Bridge.context, uri))
            DC.buildDocumentUriUsingTree(uri, DC.getTreeDocumentId(uri)) else uri
    }
    fun persist(uri: Uri, flags: Int) {
        check(flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0) { "Read access was not granted" }
        resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    fun stat(uri: Uri): JSONObject {
        val doc = document(uri)
        val rows = DocumentIo.query(doc, doc)
        check(rows.length() == 1) { "Document is unavailable; select its folder again" }
        return rows.getJSONObject(0)
    }
    fun children(uri: Uri): JSONArray {
        val doc = document(uri)
        check(DC.isTreeUri(doc)) { "Choose a folder to access its children" }
        return DocumentIo.query(DC.buildChildDocumentsUriUsingTree(doc, DC.getDocumentId(doc)), doc)
    }
    fun relative(root: Uri, path: String): JSONObject {
        var current = stat(root)
        if (path.isEmpty()) return current
        for (name in path.split('/')) {
            require(name.isNotEmpty() && name != "." && name != "..") { "Invalid relative document name" }
            check(current.getBoolean("directory")) { "Expected a folder" }
            val rows = children(Uri.parse(current.getString("uri")))
            val matches = (0 until rows.length()).map { rows.getJSONObject(it) }.filter { it.getString("name") == name }
            check(matches.size == 1) { "Missing or ambiguous document: $name" }
            current = matches.single()
        }
        return current
    }
    fun probe(uri: Uri): JSONObject {
        val result = stat(uri)
        check(!result.getBoolean("directory") && !result.getBoolean("virtual")) { "Not a readable audio document" }
        try {
            DocumentIo.read(document(uri)) { fd ->
                Os.lseek(fd.fileDescriptor, 0, OsConstants.SEEK_CUR)
                result.put("seekable", true)
                val duration = AudioProbe.inspect(fd) ?: run {
                    val reader = MediaMetadataRetriever()
                    try {
                        if (fd.declaredLength >= 0) reader.setDataSource(fd.fileDescriptor, fd.startOffset, fd.declaredLength)
                        else {
                            check(fd.startOffset == 0L) { "Unknown length of document slice" }
                            reader.setDataSource(fd.fileDescriptor)
                        }
                        reader.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
                    } finally { reader.release() }
                }
                check(duration != null && duration > 0) { "Unsupported or damaged audio document" }
                result.put("duration", duration)
            }
        } catch (error: Exception) {
            throw IllegalArgumentException(accessError(error) ?: "This audio file is damaged or not supported on this device.", error)
        }
        return result
    }
    /** A duplicated FD is transferred to Rust exactly once; offsets stay explicit. */
    private fun open(uri: Uri): JSONObject {
        return DocumentIo.read(document(uri)) { asset ->
            Os.lseek(asset.fileDescriptor, 0, OsConstants.SEEK_CUR)
            val result = JSONObject().put("offset", asset.startOffset)
                .put("length", if (asset.declaredLength < 0) JSONObject.NULL else asset.declaredLength)
            val fd = ParcelFileDescriptor.dup(asset.fileDescriptor)
            result.put("fd", fd.detachFd())
        }
    }
    fun request(message: String): String {
        return try {
            check(Looper.myLooper() != Looper.getMainLooper()) { "Provider I/O must run off the UI thread" }
            val request = JSONObject(message)
            val uri = Uri.parse(request.getString("uri"))
            val value: Any = when (request.getString("op")) {
                "stat" -> stat(uri)
                "children" -> children(uri)
                "relative" -> relative(uri, request.getString("relative"))
                "probe" -> probe(uri)
                "open" -> open(uri)
                else -> error("Unknown document operation")
            }
            JSONObject().put("value", value).toString()
        } catch (error: Exception) {
            android.util.Log.w("CarlitosDocuments", "Provider request failed", error)
            JSONObject().put("error", accessError(error) ?: error.message ?: "Cannot read this source. Select its folder again.").toString()
        }
    }
}
