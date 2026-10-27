package io.github.mny315.carlitos

import android.app.Application
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.view.HapticFeedbackConstants
import org.json.JSONObject
import java.lang.ref.WeakReference

class CarlitosApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        Bridge.context = this
        Bridge.initialize(this)
    }
}

/** Only application context and a weak Activity reference cross lifecycle boundaries. */
object Bridge {
    init { System.loadLibrary("carlitos") }
    lateinit var context: Context
    val main = Handler(Looper.getMainLooper())
    var activity = WeakReference<MainActivity>(null)
    var service: PlaybackService? = null
    private val pending = ArrayDeque<JSONObject>()
    private var audioToken = 0L
    private var syncMulticast: android.net.wifi.WifiManager.MulticastLock? = null
    private val startupTimeout = Runnable {
        if (service == null && pending.isNotEmpty()) {
            audioFailed(IllegalStateException("Playback service did not start"))
        }
    }
    @JvmStatic fun documents(message: String): String = Documents.request(message)
    @JvmStatic fun hasLocalNetwork(): Boolean {
        val connectivity = context.getSystemService(Context.CONNECTIVITY_SERVICE) as android.net.ConnectivityManager
        return connectivity.allNetworks.any { network ->
            val capabilities = connectivity.getNetworkCapabilities(network)
            capabilities?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true ||
                capabilities?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_ETHERNET) == true
        }
    }
    @JvmStatic fun deviceName(): String {
        val name = runCatching {
            android.provider.Settings.Global.getString(context.contentResolver, "device_name")
        }.getOrNull()?.trim()
        if (!name.isNullOrBlank()) return name
        val model = android.os.Build.MODEL.trim()
        val manufacturer = android.os.Build.MANUFACTURER.trim()
        return if (model.startsWith(manufacturer, ignoreCase = true)) model else "$manufacturer $model"
    }
    @JvmStatic external fun initialize(context: Context)
    @JvmStatic external fun event(message: String)
    @JvmStatic fun voiceBands(): Long = VoiceLevel.readBands()
    @JvmStatic fun setVisualizerEnabled(enabled: Boolean) { VoiceLevel.enabled = enabled }
    fun emit(op: String, vararg fields: Pair<String, Any?>) {
        val event = JSONObject().put("op", op)
        fields.forEach { (key, value) -> event.put(key, value ?: JSONObject.NULL) }
        event(event.toString())
    }
    @JvmStatic fun dispatch(message: String) {
        main.post {
            try {
                val command = JSONObject(message)
                if (command.getString("op") == "haptic") {
                    activity.get()?.window?.decorView?.performHapticFeedback(
                        when (command.optString("kind")) {
                            "scale-tick" -> if (android.os.Build.VERSION.SDK_INT >= 34) HapticFeedbackConstants.SEGMENT_FREQUENT_TICK
                                else if (android.os.Build.VERSION.SDK_INT >= 27) HapticFeedbackConstants.TEXT_HANDLE_MOVE
                                else HapticFeedbackConstants.CLOCK_TICK
                            "tick" -> HapticFeedbackConstants.CLOCK_TICK
                            "playback" -> HapticFeedbackConstants.VIRTUAL_KEY
                            else -> HapticFeedbackConstants.KEYBOARD_TAP
                        })
                } else if (command.getString("op") == "background") {
                    activity.get()?.moveTaskToBack(true)
                } else if (command.getString("op") == "system-bars") {
                    activity.get()?.updateSystemBars(command.getBoolean("dark"))
                } else if (command.getString("op") == "battery-settings") {
                    val current = activity.get()
                    if (current != null && !BackgroundPlayback.openSettings(current)) {
                        emit("error", "message" to "Could not open Android battery settings")
                    }
                } else if (command.getString("op") == "pick") {
                    activity.get()?.pick(command.optString("kind", "file"), command.optString("owner"))
                        ?: emit("error", "message" to "Open the app to select an audio file")
                } else if (command.getString("op") == "sync-scan") {
                    activity.get()?.scanSyncQr(command.getString("prompt"))
                } else if (command.getString("op") == "sync-network") {
                    if (command.getBoolean("enabled")) {
                        if (syncMulticast == null) {
                            val wifi = context.getSystemService(Context.WIFI_SERVICE) as android.net.wifi.WifiManager
                            syncMulticast = wifi.createMulticastLock("carlitos-sync").apply { setReferenceCounted(false); acquire() }
                        }
                    } else {
                        syncMulticast?.let { if (it.isHeld) it.release() }
                        syncMulticast = null
                    }
                } else {
                    audio(command)
                }
            } catch (error: Exception) {
                emit("error", "message" to (error.message ?: error.toString()))
            }
        }
    }
    private fun audio(command: JSONObject) {
        if (command.has("token")) audioToken = command.getLong("token")
        try {
            val current = service
            if (current != null) current.command(command)
            else if (pending.isEmpty() && (command.getString("op") == "snapshot" || command.getString("op") == "stop")) {
                // Lifecycle checkpoints must not create an idle audio service.
                // Android may have released it while the app was in background.
                // Acknowledge its absence so Rust saves its confirmed position
                // and can also complete a pending shutdown without a timeout.
                // Keep the request token: a new load may already be queued in Rust.
                emit("audio_absent", "token" to audioToken)
            }
            else {
                check(pending.size < 64) { "Playback service did not start" }
                pending.addLast(command)
                if (pending.size == 1) {
                    check(context.startService(Intent(context, PlaybackService::class.java)) != null) {
                        "Playback service is unavailable"
                    }
                    main.postDelayed(startupTimeout, 5000)
                }
            }
        } catch (error: Exception) { audioFailed(error) }
    }
    private fun audioFailed(error: Exception) {
        // A later retry must not replay an earlier failed load/seek request.
        pending.clear()
        main.removeCallbacks(startupTimeout)
        emit("audio_failed", "token" to audioToken, "message" to (error.message ?: error.toString()))
    }
    fun ready(service: PlaybackService) {
        main.removeCallbacks(startupTimeout)
        this.service = service
        while (pending.isNotEmpty()) service.command(pending.removeFirst())
    }
}
