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
    private val startupTimeout = Runnable {
        if (service == null && pending.isNotEmpty()) {
            audioFailed(IllegalStateException("Playback service did not start"))
        }
    }
    @JvmStatic fun documents(message: String): String = Documents.request(message)
    @JvmStatic external fun initialize(context: Context)
    @JvmStatic external fun event(message: String)
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
                            else -> HapticFeedbackConstants.KEYBOARD_TAP
                        })
                } else if (command.getString("op") == "background") {
                    activity.get()?.moveTaskToBack(true)
                } else if (command.getString("op") == "system-bars") {
                    activity.get()?.updateSystemBars(command.getBoolean("dark"))
                } else if (command.getString("op") == "pick") {
                    activity.get()?.pick(command.optString("kind", "file"), command.optString("owner"))
                        ?: emit("error", "message" to "Open the app to select an audio file")
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
