package io.github.mny315.carlitos

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.Process

/** Debug-only process-death fixture; cannot stop the user's application. */
class TestProcessReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (context.packageName != "io.github.mny315.carlitos.playbacktest" ||
            intent.action != "io.github.mny315.carlitos.test.SIGKILL" ||
            intent.getIntExtra("expected-pid", -1) != Process.myPid()) return
        // Let the shell broadcast finish, then send SIGKILL from the app's own
        // SELinux domain. Some devices prohibit signals from `run-as`.
        Handler(Looper.getMainLooper()).postDelayed({ Process.killProcess(Process.myPid()) }, 200)
    }
}
