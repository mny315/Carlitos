package io.github.mny315.carlitos

import android.app.Instrumentation
import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import android.provider.DocumentsContract as DC
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import org.json.JSONObject

@UnstableApi
class ImportSuite(private val instrumentation: Instrumentation) {
    private external fun nativeSuite(request: String): String
    private fun note(message: String) = instrumentation.sendStatus(0, Bundle().apply { putString("stream", "PASS $message\n") })
    private fun call(request: JSONObject): JSONObject {
        val result = JSONObject(nativeSuite(request.toString()))
        check(!result.has("error")) { result.optString("error") }
        return result
    }
    fun benchmark(rootName: String) {
        val grants = Bridge.context.contentResolver.persistedUriPermissions.filter { it.isReadPermission && DC.isTreeUri(it.uri) }
        val grant = grants.firstOrNull {
            runCatching { Documents.stat(it.uri).getString("name") == rootName }.getOrDefault(false)
        } ?: error("Select the benchmark folder first: $rootName")
        val result = call(JSONObject().put("benchmark", true).put("root", grant.uri))
        instrumentation.sendStatus(0, Bundle().apply { putString("stream", "IMPORT BENCHMARK $result\n") })
    }
    fun run() {
        grantFixtures(instrumentation)
        val authority = "io.github.mny315.carlitos.test.sources"
        fun tree(id: String): Uri = DC.buildDocumentUriUsingTree(DC.buildTreeDocumentUri(authority, id.substringBefore('/')), id)
        val bulk = call(JSONObject().put("bulk", tree("s4:bulk")))
        note("437 synthetic MP3 documents scanned in ${bulk.getLong("scan_ms")} ms; order, duration and shared cover preserved")
        val result = call(JSONObject().put("root", tree("s4:old"))
            .put("moved", tree("s4:moved/Автор/Книга")).put("bad", tree("s4:bad/Автор/Книга")))
        val checks = result.getJSONArray("checks")
        for (index in 0 until checks.length()) note(checks.getString(index))
        for (type in listOf("mp3", "m4b")) {
            val slice = call(JSONObject().put("document", DC.buildDocumentUri("io.github.mny315.carlitos.test.slices", "slice-$type")))
            if (type == "mp3") check(slice.getString("title") == "Глава 1.1" && slice.getInt("year") == 2024 && !slice.isNull("cover"))
            else check(slice.getJSONArray("chapters").length() == 2 && slice.getString("artist") == "Автор глав")
        }
        note("real provider FD offsets/lengths for MP3 tags, cover and QuickTime chapters")
        val files = result.getJSONArray("files")
        var player: ExoPlayer? = null
        try {
            instrumentation.runOnMainSync { player = ExoPlayer.Builder(instrumentation.targetContext).build().apply { volume = 0f } }
            for (index in 0 until files.length()) {
                val file = files.getJSONObject(index)
                instrumentation.runOnMainSync {
                    player!!.setMediaItem(MediaItem.fromUri(file.getString("uri")))
                    player!!.prepare()
                    player!!.play()
                }
                val deadline = SystemClock.elapsedRealtime() + 10000
                var decoded = false
                while (SystemClock.elapsedRealtime() < deadline) {
                    instrumentation.runOnMainSync {
                        check(player!!.playerError == null) { "${file.getString("relative")}: ${player!!.playerError}" }
                        decoded = player!!.currentPosition >= 200 || player!!.playbackState == Player.STATE_ENDED
                    }
                    if (decoded) break
                    Thread.sleep(25)
                }
                check(decoded) { "Playback failed: ${file.getString("relative")}" }
                note("Media3 playback ${file.getString("relative")}")
            }
        } finally { instrumentation.runOnMainSync { player?.release() } }
        // If selected, repeat the import/SQLite suite through the actual system provider.
        val grants = Bridge.context.contentResolver.persistedUriPermissions.filter { it.isReadPermission && DC.isTreeUri(it.uri) }
        var real = false
        for (grant in grants) {
            val root = runCatching { Documents.stat(grant.uri) }.getOrNull() ?: continue
            if (root.getString("name") != "Carlitos-import") continue
            call(JSONObject().put("root", grant.uri))
            real = true
        }
        if (real) note("actual persisted SAF folder import, overlap, rescan, checkpoint and source removal")
        else instrumentation.sendStatus(0, Bundle().apply { putString("stream", "SKIP real SAF folder: select Download/Carlitos-import and rerun\n") })
    }
}
