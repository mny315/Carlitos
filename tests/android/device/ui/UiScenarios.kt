package io.github.mny315.carlitos

import android.content.Intent
import android.content.pm.ActivityInfo
import androidx.media3.common.util.UnstableApi

@UnstableApi
internal fun UiSuite.run(libraryOnly: Boolean = false, chaptersOnly: Boolean = false, inputOnly: Boolean = false) {
    check(instrumentation.targetContext.packageName.endsWith(".playbacktest"))
    val originalFont = shell("settings get system font_scale")
    val originalSize = shell("wm size").lineSequence().firstOrNull { it.startsWith("Override size:") }?.substringAfter(":")?.trim()
    val originalNight = shell("cmd uimode night").substringAfter("Night mode: ").trim()
    val packageName = instrumentation.targetContext.packageName
    // Keep this playable book before the synthetic "Swipe fixture" rows.
    val title = "Android UI — Очень длинное название аудиокниги с главами и продолжением истории"
    try {
        shell("input keyevent KEYCODE_WAKEUP")
        shell("wm dismiss-keyguard")
        cancelTouches()
        instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        main { Bridge.activity.get()!!.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        eventually("UI startup") { try { call().getDouble("width") > 0 } catch (_: Exception) { false } }
        grantFixtures(instrumentation, ui = true)
        if (!libraryOnly && !chaptersOnly && !inputOnly) DocumentChecks(instrumentation).apply { fixtures(); slice(); largeAudioPacket(); timeouts() }
        PlaybackSuite(instrumentation).fixture(title, listOf("tone", "part2"))
        call("setup")
        if (inputOnly) {
            imeSelectionAndComposition(title)
            return
        }
        if (chaptersOnly) {
            chapterPlaybackChecks(title)
            pauseResponsivenessChecks()
            mouseWheelChecks()
            return
        }
        if (!libraryOnly) {
            imeSelectionAndComposition(title)
            stalePickerResults(title)
            recreatedRemovedBook()
            detachedNativeCallbacks()
            recreatedSources()
            recreatedEdits(title)
            displayScaleChecks()
            feedbackChecks()
        }
        bookSwipeChecks(title)
        if (libraryOnly) return
        chapterPlaybackChecks(title)
        pauseResponsivenessChecks()
        mouseWheelChecks()
        presentationChecks(title, packageName)
    } finally {
        cancelTouches()
        main { Bridge.activity.get()?.window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        shell(if (originalFont == "null") "settings delete system font_scale" else "settings put system font_scale $originalFont")
        main { Bridge.activity.get()?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED }
        shell(if (originalSize == null) "wm size reset" else "wm size $originalSize")
        if (originalNight in listOf("yes", "no", "auto", "custom")) shell("cmd uimode night $originalNight")
        shell("cmd locale set-app-locales $packageName")
        Bridge.emit("play", "value" to false)
        shell("input keyevent KEYCODE_SLEEP")
    }
}
