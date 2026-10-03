package io.github.mny315.carlitos

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.provider.DocumentsContract

/** Runs in the fixture APK's UID so it can grant access to its own provider. */
class FixtureGrantActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        val authority = "io.github.mny315.carlitos.test.sources"
        val ui = intent.getBooleanExtra("ui", false)
        val isolated = ui || intent.getBooleanExtra("isolated", false)
        val target = "io.github.mny315.carlitos" + if (isolated) ".playbacktest" else ""
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION
        val documents = if (ui) listOf("audio", "pipe", "denied", "missing", "stall-open", "stall-probe")
            else listOf("audio", "pipe", "denied", "missing")
        val trees = if (ui) listOf("slow", "root", "duplicates")
            else listOf("root", "duplicates", "s4:old", "s4:moved", "s4:bad", "s4:bulk")
        val slices = if (ui) listOf("slice-wav") else listOf("slice-mp3", "slice-m4b")
        documents.forEach { grantUriPermission(target, DocumentsContract.buildDocumentUri(authority, it), flags) }
        trees.forEach { grantUriPermission(target, DocumentsContract.buildTreeDocumentUri(authority, it), flags) }
        slices.forEach {
            grantUriPermission(target, DocumentsContract.buildDocumentUri("io.github.mny315.carlitos.test.slices", it), flags)
        }
        finish()
    }
}
