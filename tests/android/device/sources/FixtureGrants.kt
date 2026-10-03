package io.github.mny315.carlitos

import android.app.Instrumentation
import android.content.Intent
import android.content.pm.PackageManager
import android.os.ParcelFileDescriptor
import android.os.Process
import android.os.SystemClock
import android.provider.DocumentsContract

/** Wait for the final URI grant, not for a window that immediately finishes. */
internal fun grantFixtures(instrumentation: Instrumentation, ui: Boolean = false) {
    val isolated = instrumentation.targetContext.packageName.endsWith(".playbacktest")
    val component = "io.github.mny315.carlitos.fixtures/io.github.mny315.carlitos.FixtureGrantActivity"
    // `am start -W` can wait forever on Android 11 when onCreate calls finish().
    instrumentation.uiAutomation.executeShellCommand("am start -n $component --ez ui $ui --ez isolated $isolated").use {
        ParcelFileDescriptor.AutoCloseInputStream(it).use { input -> input.readBytes() }
    }
    val last = DocumentsContract.buildDocumentUri("io.github.mny315.carlitos.test.slices", if (ui) "slice-wav" else "slice-m4b")
    val deadline = SystemClock.elapsedRealtime() + 10000
    while (instrumentation.targetContext.checkUriPermission(last, Process.myPid(), Process.myUid(),
            Intent.FLAG_GRANT_READ_URI_PERMISSION) != PackageManager.PERMISSION_GRANTED) {
        check(SystemClock.elapsedRealtime() < deadline) { "Fixture provider did not grant access to $last" }
        Thread.sleep(50)
    }
}
