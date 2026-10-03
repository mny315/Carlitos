package io.github.mny315.carlitos

import android.app.Instrumentation
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract as DC

class SourcesSuite(private val instrumentation: Instrumentation) {
    private fun note(message: String) = instrumentation.sendStatus(0, Bundle().apply { putString("stream", "PASS $message\n") })
    fun run(revoke: Boolean) {
        grantFixtures(instrumentation)
        DocumentChecks(instrumentation).fixtures()
        // This grant must come from the actual system picker, not the fixture provider.
        val grants = Bridge.context.contentResolver.persistedUriPermissions.filter { it.isReadPermission && DC.isTreeUri(it.uri) }
        check(grants.isNotEmpty()) { "Select the Carlitos-sources test folder using the real SAF picker first" }
        var checked = 0
        for (grant in grants) {
            val root = Documents.stat(grant.uri)
            if (root.getString("name") != "Carlitos-sources") continue
            val entries = Documents.children(grant.uri)
            check(entries.length() > 0)
            val first = Documents.relative(grant.uri, "Книга/CD 1/01.wav")
            check(Documents.probe(Uri.parse(first.getString("uri"))).getLong("duration") > 0)
            if (revoke) {
                Bridge.context.contentResolver.releasePersistableUriPermission(grant.uri, android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)
                check(runCatching { Documents.stat(grant.uri) }.isFailure) { "Revoked tree remains readable" }
                note("real fixture tree grant revoked; source must be reselected")
            }
            checked++
        }
        check(checked > 0) { "No persisted Carlitos-sources fixture folder grant" }
        note("real persisted folder access and nested audio read after process restart")
    }
}
