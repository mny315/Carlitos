package io.github.mny315.carlitos

import android.app.Activity
import android.app.Instrumentation
import android.content.ComponentName
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Bundle
import android.os.SystemClock
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import java.util.concurrent.TimeUnit

/** Device behavior tests. Requires one local document selected using the real SAF picker. */
@UnstableApi
class DeviceInstrumentation : Instrumentation() {
    private lateinit var controller: MediaController
    private fun main(action: () -> Unit) = runOnMainSync(action)
    private fun checkEventually(description: String, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 8000
        while (SystemClock.elapsedRealtime() < deadline) {
            var success = false
            main { success = condition() }
            if (success) return
            Thread.sleep(50)
        }
        error(description)
    }
    private fun note(message: String) {
        sendStatus(0, Bundle().apply { putString("stream", "$message\n") })
    }
    private var sources = false
    private var imports = false
    private var playback = false
    private var ui = false
    private var libraryOnly = false
    private var chaptersOnly = false
    private var inputOnly = false
    private var timedSwipe: String? = null
    private var revoke = false
    private var importBenchmark: String? = null
    private var soakMinutes = 0
    private var formatsOnly = false
    private var formatPlayMs = 150L
    private var settingsRecovery = false
    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        sources = arguments?.getString("suite") == "sources"
        imports = arguments?.getString("suite") == "import"
        playback = arguments?.getString("suite") == "playback"
        ui = arguments?.getString("suite") == "ui"
        libraryOnly = arguments?.getString("library-only") == "true"
        chaptersOnly = arguments?.getString("chapters-only") == "true"
        inputOnly = arguments?.getString("input-only") == "true"
        timedSwipe = arguments?.getString("timed-swipe")
        revoke = arguments?.getString("revoke") == "true"
        importBenchmark = arguments?.getString("import-benchmark")
        soakMinutes = arguments?.getString("soak-minutes")?.toInt() ?: 0
        formatsOnly = arguments?.getString("formats-only") == "true"
        formatPlayMs = arguments?.getString("format-play-ms")?.toLong() ?: 150L
        settingsRecovery = arguments?.getString("settings-recovery") == "true"
        start()
    }
    override fun onStart() {
        try {
            if (settingsRecovery) {
                UiSuite(this).settingsRecoveryNotice()
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android settings recovery notice\n") })
                return
            }
            if (formatsOnly) {
                PlaybackSuite(this).run(true, formatPlayMs)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android audio format tests\n") })
                return
            }
            if (soakMinutes != 0) {
                PlaybackSuite(this).soak(soakMinutes)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android playback soak\n") })
                return
            }
            // Reproducible input for SurfaceFlinger frame timing recordings. Start
            // the activity to measure first, then pass x0,y0,x1,y1,duration_ms.
            // Unlike `input swipe`, this does not flood the queue between frames.
            timedSwipe?.let { spec ->
                val p = spec.split(',').map { it.toFloat() }
                require(p.size == 5 && p.all { it.isFinite() } && p[4] in 8f..10_000f)
                val down = SystemClock.uptimeMillis()
                fun send(action: Int, fraction: Float) {
                    val event = android.view.MotionEvent.obtain(down, SystemClock.uptimeMillis(), action,
                        p[0] + (p[2] - p[0]) * fraction, p[1] + (p[3] - p[1]) * fraction, 0)
                    event.source = android.view.InputDevice.SOURCE_TOUCHSCREEN
                    check(uiAutomation.injectInputEvent(event, false))
                    event.recycle()
                }
                send(android.view.MotionEvent.ACTION_DOWN, 0f)
                for (t in 8..p[4].toInt() step 8) {
                    val wait = down + t - SystemClock.uptimeMillis()
                    if (wait > 0) Thread.sleep(wait)
                    send(android.view.MotionEvent.ACTION_MOVE, t / p[4])
                }
                send(android.view.MotionEvent.ACTION_UP, 1f)
                Thread.sleep(400)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS timed swipe\n") })
                return
            }
            importBenchmark?.let {
                ImportSuite(this).benchmark(it)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android import benchmark\n") })
                return
            }
            if (ui) {
                UiSuite(this).run(libraryOnly, chaptersOnly, inputOnly)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android UI tests\n") })
                return
            }
            if (playback) {
                PlaybackSuite(this).run()
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android playback tests\n") })
                return
            }
            if (imports) {
                ImportSuite(this).run()
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android import tests\n") })
                return
            }
            if (sources) {
                SourcesSuite(this).run(revoke)
                finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android source tests\n") })
                return
            }
            startActivitySync(Intent(targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            lateinit var future: com.google.common.util.concurrent.ListenableFuture<MediaController>
            main { future = MediaController.Builder(targetContext,
                SessionToken(targetContext, ComponentName(targetContext, PlaybackService::class.java))).buildAsync() }
            controller = future.get(10, TimeUnit.SECONDS)
            checkEventually("No selected file restored") { controller.mediaItemCount == 1 && controller.playbackState == Player.STATE_READY }
            checkEventually("Cold launch unexpectedly played") { !controller.playWhenReady }
            main { controller.play() }
            checkEventually("System play did not reach Rust/player") { controller.isPlaying }
            main { controller.pause() }
            checkEventually("System pause failed") { !controller.playWhenReady }
            main { for (position in listOf(4000L, 22000L, 7000L, 16000L)) controller.seekTo(position) }
            checkEventually("Rapid seeks did not settle at final position") { kotlin.math.abs(controller.currentPosition - 16000L) < 500 }
            note("PASS session play/pause and rapid seeks")
            main { controller.seekToNextMediaItem(); controller.seekToPreviousMediaItem() }
            checkEventually("Single-file next/previous replaced the item") { controller.mediaItemCount == 1 && !controller.playWhenReady }
            main { controller.play() }
            checkEventually("Play before focus test failed") { controller.isPlaying }
            val audio = targetContext.getSystemService(AudioManager::class.java)
            val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
                .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
                .setOnAudioFocusChangeListener { }.build()
            check(audio.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) { "Focus request denied" }
            checkEventually("Transient focus loss did not suppress playback") { !controller.isPlaying }
            audio.abandonAudioFocusRequest(focus)
            checkEventually("Focus return did not resume playback") { controller.isPlaying }
            note("PASS transient call-like audio focus loss/return")
            val permanent = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setOnAudioFocusChangeListener { }.build()
            check(audio.requestAudioFocus(permanent) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED)
            checkEventually("Permanent focus loss did not pause") { !controller.playWhenReady }
            audio.abandonAudioFocusRequest(permanent)
            Thread.sleep(300)
            main { check(!controller.playWhenReady); controller.play() }
            checkEventually("Resume after permanent focus loss failed") { controller.isPlaying }
            note("PASS permanent audio focus loss")
            // The platform's noisy broadcast exercises ExoPlayer's actual receiver.
            val noisyResult = uiAutomation.executeShellCommand("am broadcast -a android.media.AUDIO_BECOMING_NOISY -p ${targetContext.packageName}").use { descriptor ->
                android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { String(it.readBytes()) }
            }
            if (noisyResult.contains("Broadcast completed")) {
                checkEventually("Noisy route change did not pause playback") { !controller.playWhenReady }
                note("PASS audio becoming noisy broadcast")
            } else {
                note("SKIP physical headset disconnection: OS rejects synthetic protected broadcast")
            }
            main { controller.play() }
            checkEventually("Play before recreation failed") { controller.isPlaying }
            var service: PlaybackService? = null
            var previous: MainActivity? = null
            main { service = Bridge.service; previous = Bridge.activity.get(); check(previous != null); previous!!.recreate() }
            checkEventually("Activity did not recreate") { Bridge.activity.get() != null && Bridge.activity.get() !== previous }
            checkEventually("Playback did not survive Activity recreation") { controller.isPlaying && Bridge.service === service }
            note("PASS Activity recreation with the same service/player")
            main { controller.pause() }
            checkEventually("Final pause failed") { !controller.playWhenReady }
            main { controller.release() }
            finish(Activity.RESULT_OK, Bundle().apply { putString("stream", "PASS Android session tests\n") })
        } catch (error: Throwable) {
            finish(Activity.RESULT_CANCELED, Bundle().apply { putString("stream", "FAIL ${error.stackTraceToString()}\n") })
        }
    }
}
