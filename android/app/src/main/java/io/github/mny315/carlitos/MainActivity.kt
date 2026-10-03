package io.github.mny315.carlitos

import android.app.NativeActivity
import android.content.Intent
import android.os.Bundle
import android.os.Build
import android.content.res.Configuration
import android.view.Surface
import android.view.SurfaceHolder
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import java.lang.ref.WeakReference
import java.util.concurrent.Executors

class MainActivity : NativeActivity() {
    private var backCallback: OnBackInvokedCallback? = null
    private val pickerOwners = mutableMapOf<Int, String>()
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        for (code in 1..4) state?.getString("carlitos.pickerOwner.$code")?.let { pickerOwners[code] = it }
        Bridge.activity = WeakReference(this)
        if (Build.VERSION.SDK_INT >= 33) {
            backCallback = OnBackInvokedCallback { Bridge.emit("back") }.also {
                onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, it)
            }
        }
    }
    override fun onSaveInstanceState(state: Bundle) {
        super.onSaveInstanceState(state)
        pickerOwners.forEach { (code, owner) -> state.putString("carlitos.pickerOwner.$code", owner) }
    }
    override fun onResume() {
        super.onResume()
        Bridge.activity = WeakReference(this)
        reportConfiguration(resources.configuration)
        Bridge.emit("hidden", "value" to false)
        updateFrameRate()
    }
    @Suppress("DEPRECATION")
    private fun updateFrameRate(surface: Surface? = null) {
        val display = windowManager.defaultDisplay
        val current = display.mode
        val rate = display.supportedModes.filter {
            it.physicalWidth == current.physicalWidth && it.physicalHeight == current.physicalHeight
        }.maxOfOrNull { it.refreshRate } ?: current.refreshRate
        // Native rendering has no View animation to request a high refresh rate.
        // Keep the resolution and let Android apply its power/thermal policy.
        if (window.attributes.preferredRefreshRate != rate) {
            window.attributes = window.attributes.apply { preferredRefreshRate = rate }
        }
        if (Build.VERSION.SDK_INT >= 30 && surface?.isValid == true) {
            surface.setFrameRate(rate, Surface.FRAME_RATE_COMPATIBILITY_DEFAULT)
        }
    }
    override fun surfaceCreated(holder: SurfaceHolder) {
        super.surfaceCreated(holder)
        updateFrameRate(holder.surface)
    }
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        super.surfaceChanged(holder, format, width, height)
        updateFrameRate(holder.surface)
    }
    override fun onStop() {
        Bridge.emit("hidden", "value" to true)
        super.onStop()
    }
    override fun onDestroy() {
        if (Build.VERSION.SDK_INT >= 33) backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
        if (Bridge.activity.get() === this) Bridge.activity.clear()
        super.onDestroy()
    }
    @Deprecated("Fallback for Android 8–12; newer versions use OnBackInvokedCallback")
    override fun onBackPressed() { Bridge.emit("back") }

    @Suppress("DEPRECATION")
    fun updateSystemBars(dark: Boolean) {
        // Preserve the layout flags installed by Slint, changing icon contrast only.
        val decor = window.decorView
        val light = android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
        decor.systemUiVisibility = if (dark) decor.systemUiVisibility and light.inv() else decor.systemUiVisibility or light
    }
    override fun onConfigurationChanged(configuration: Configuration) {
        super.onConfigurationChanged(configuration)
        reportConfiguration(configuration)
    }
    private fun reportConfiguration(configuration: Configuration) {
        Bridge.emit("configuration",
            "language" to configuration.locales[0].language,
            "dark" to (configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES),
            "font_scale" to configuration.fontScale)
    }
    fun pick(kind: String, owner: String) {
        val code = when (kind) { "folder" -> 2; "source" -> 3; "cover" -> 4; else -> 1 }
        pickerOwners[code] = owner
        startActivityForResult(Intent(if (code == 2 || code == 3) Intent.ACTION_OPEN_DOCUMENT_TREE else Intent.ACTION_OPEN_DOCUMENT).apply {
            if (code == 1 || code == 4) {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = if (code == 4) "image/*" else "audio/*"
            }
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        }, code)
    }
    @Deprecated("NativeActivity uses the platform result callback")
    override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request, result, data)
        // Capture before provider work: the user may open another editor
        // while stat/probe runs, and this Activity may be recreated meanwhile.
        val owner = pickerOwners.remove(request).orEmpty()
        if (request !in 1..4 || result != RESULT_OK) return
        val uri = data?.data ?: return
        val flags = data.flags
        // The worker retains no Activity. Cancellation leaves library state intact.
        documents.execute {
            try {
                Documents.persist(uri, flags)
                val doc = if (request == 1) Documents.probe(uri) else Documents.stat(uri)
                if (request == 2 || request == 3) check(doc.getBoolean("directory")) { "Choose a folder" }
                if (request == 1) Bridge.emit("document", "document" to doc)
                else Bridge.emit("picked", "kind" to when (request) { 2 -> "folder"; 3 -> "source"; else -> "cover" },
                    "uri" to doc.getString("uri"), "name" to doc.getString("name"), "owner" to owner)
            } catch (error: Exception) {
                Bridge.emit("error", "message" to (Documents.accessError(error) ?: error.message ?: error.toString()))
            }
        }
    }
    companion object {
        private val documents = Executors.newSingleThreadExecutor { work -> Thread(work, "carlitos-documents") }
    }
}
