package io.github.mny315.carlitos

import android.app.Instrumentation
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.provider.DocumentsContract as DC
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Provider regressions also run in the isolated UI-test package. */
internal class DocumentChecks(private val instrumentation: Instrumentation) {
    private fun note(message: String) = instrumentation.sendStatus(0, Bundle().apply { putString("stream", "PASS $message\n") })
    fun largeAudioPacket() {
        val uri = Uri.parse("content://io.github.mny315.carlitos.test.playback/document/flac-large")
        val probed = Documents.probe(uri)
        check(probed.getLong("duration") == 2000L && probed.getBoolean("seekable"))
        note("FLAC with a 393216-byte PCM packet imports without a fixed-buffer failure")
    }
    fun fixtures() {
        val authority = "io.github.mny315.carlitos.test.sources"
        fun uri(id: String) = DC.buildDocumentUri(authority, id)
        val unknown = Documents.stat(uri("audio"))
        check(unknown.isNull("size") && unknown.isNull("modified"))
        val missing = Documents.stat(uri("missing"))
        check(missing.isNull("size") && missing.isNull("modified"))
        check(missing.getString("name") == "missing")
        check(unknown.getString("name") == "Глава + 1.wav")
        val probed = Documents.probe(uri("audio"))
        check(probed.getLong("duration") == 1000L && probed.getBoolean("seekable"))
        note("unknown size/date and seekable audio with Cyrillic name")
        val opened = JSONObject(Documents.request(JSONObject().put("op", "open").put("uri", uri("audio")).toString())).getJSONObject("value")
        ParcelFileDescriptor.AutoCloseInputStream(ParcelFileDescriptor.adoptFd(opened.getInt("fd"))).use {
            check(String(it.readNBytes(4)) == "RIFF")
        }
        note("owned descriptor reads without copying the recording")
        val pipe = JSONObject(Documents.request(JSONObject().put("op", "probe").put("uri", uri("pipe")).toString()))
        check(pipe.has("error"))
        val denied = JSONObject(Documents.request(JSONObject().put("op", "stat").put("uri", uri("denied")).toString()))
        check(denied.getString("error").contains("revoked"))
        note("nonseekable streams and denied access report errors")
        val tree = DC.buildTreeDocumentUri(authority, "root")
        check(Documents.children(tree).length() == 2)
        val child = Documents.relative(tree, "Глава + 1.wav")
        check(DC.getDocumentId(Uri.parse(child.getString("uri"))) == "audio")
        for (path in listOf("../audio", "/audio", "missing")) {
            check(runCatching { Documents.relative(tree, path) }.isFailure)
        }
        check(runCatching { Documents.relative(DC.buildTreeDocumentUri(authority, "duplicates"), "same.wav") }.isFailure)
        note("relative lookup rejects traversal, missing and ambiguous names")
        var uiRejected = false
        instrumentation.runOnMainSync {
            uiRejected = JSONObject(Documents.request(JSONObject().put("op", "stat").put("uri", uri("audio")).toString())).has("error")
        }
        check(uiRejected)
        note("provider I/O cannot run on Android main thread")
    }
    fun timeouts() {
        val executor = Executors.newFixedThreadPool(2)
        try {
            val pending = listOf("open", "probe").map { op ->
                executor.submit<Pair<JSONObject, Long>> {
                    val uri = DC.buildDocumentUri("io.github.mny315.carlitos.test.sources", "stall-$op")
                    val started = SystemClock.elapsedRealtime()
                    val result = JSONObject(Documents.request(JSONObject().put("op", op).put("uri", uri).toString()))
                    // The old implementation succeeds after the provider's own
                    // deadline. Close even that unexpected transferred FD.
                    result.optJSONObject("value")?.let { value ->
                        if (value.has("fd")) ParcelFileDescriptor.adoptFd(value.getInt("fd")).close()
                    }
                    result to (SystemClock.elapsedRealtime() - started)
                }
            }
            val results = pending.map { it.get(30, TimeUnit.SECONDS) }
            results.forEachIndexed { index, (result, elapsed) ->
                check(result.optString("error").contains("did not respond in time")) {
                    "${listOf("open", "probe")[index]} must cancel a stalled provider: $result after $elapsed ms"
                }
                check(elapsed in 14000..18000) { "Provider cancellation took $elapsed ms" }
            }
            // Cancellation must remain local to the failed request.
            check(Documents.probe(DC.buildDocumentUri("io.github.mny315.carlitos.test.sources", "audio")).getLong("duration") == 1000L)
            note("stalled document open and probe cancel after 15 seconds; later reads succeed")
        } finally { executor.shutdownNow() }
    }
    fun slice() {
        val uri = DC.buildDocumentUri("io.github.mny315.carlitos.test.slices", "slice-wav")
        check(Documents.probe(uri).getLong("duration") == 60000L)
        val opened = JSONObject(Documents.request(JSONObject().put("op", "open").put("uri", uri).toString())).getJSONObject("value")
        ParcelFileDescriptor.adoptFd(opened.getInt("fd")).use { fd ->
            check(opened.getLong("offset") == 137L && opened.getLong("length") == 44L + 48000 * 2 * 60)
            android.system.Os.lseek(fd.fileDescriptor, opened.getLong("offset"), android.system.OsConstants.SEEK_SET)
            ParcelFileDescriptor.AutoCloseInputStream(fd).use { check(String(it.readNBytes(4)) == "RIFF") }
        }
        note("direct provider access preserves descriptor slice offsets and audio duration")
    }
}
