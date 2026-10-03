package io.github.mny315.carlitos

import android.app.PendingIntent
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingSimpleBasePlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import org.json.JSONObject

@UnstableApi
class PlaybackService : MediaSessionService() {
    private companion object { const val SEEK_STEP_MS = 15000L }
    private lateinit var player: ExoPlayer
    private var session: MediaSession? = null
    private var refreshSessionPosition: (() -> Unit)? = null
    private val handler = Handler(Looper.getMainLooper())
    private var token = 0L
    private var generation = 0L
    private var listener: Player.Listener? = null
    private var seekDone = false
    private var barrierSince: Long? = null
    private var error: String? = null
    private var applying = false
    private var loaded = false
    private var released = false
    @Volatile private var diagnostics = "{}"
    private val tick = object : Runnable {
        override fun run() {
            if (released) return
            snapshot()
        }
    }
    override fun onCreate() {
        super.onCreate()
        player = ExoPlayer.Builder(this, AudioRenderers(this))
            .setSeekBackIncrementMs(SEEK_STEP_MS)
            .setSeekForwardIncrementMs(SEEK_STEP_MS)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_SPEECH).build(), true)
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_LOCAL)
            .build()
        val routed = object : ForwardingSimpleBasePlayer(player) {
            private var publishPosition = false
            fun confirmPosition() { publishPosition = true; invalidateState() }
            // Session clients can control transport, never replace Rust's queue.
            override fun getState(): State {
                val state = super.getState()
                val commands = Player.Commands.Builder().addAll(
                    Player.COMMAND_PLAY_PAUSE, Player.COMMAND_PREPARE, Player.COMMAND_STOP,
                    Player.COMMAND_GET_CURRENT_MEDIA_ITEM, Player.COMMAND_GET_TIMELINE,
                    Player.COMMAND_GET_METADATA, Player.COMMAND_GET_VOLUME,
                    Player.COMMAND_SET_VOLUME, Player.COMMAND_SET_SPEED_AND_PITCH
                )
                if (player.isCurrentMediaItemSeekable) commands.addAll(
                    Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM, Player.COMMAND_SEEK_BACK,
                    Player.COMMAND_SEEK_FORWARD, Player.COMMAND_SEEK_TO_NEXT,
                    Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                    Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                val updated = state.buildUpon().setAvailableCommands(commands.build())
                if (publishPosition) {
                    // Controllers otherwise retain their optimistic pause clock:
                    // a clock-only audio acknowledgement emits no Player event.
                    updated.setPositionDiscontinuity(Player.DISCONTINUITY_REASON_INTERNAL, player.currentPosition)
                    publishPosition = false
                }
                return updated.build()
            }
            override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
                Bridge.emit("play", "value" to playWhenReady)
                return Futures.immediateVoidFuture()
            }
            override fun handlePrepare(): ListenableFuture<*> = Futures.immediateVoidFuture()
            override fun handleStop(): ListenableFuture<*> {
                Bridge.emit("stop")
                return Futures.immediateVoidFuture()
            }
            override fun handleSeek(index: Int, position: Long, command: Int): ListenableFuture<*> {
                when (command) {
                    Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> Bridge.emit("next", "value" to true)
                    Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> Bridge.emit("next", "value" to false)
                    Player.COMMAND_SEEK_BACK -> Bridge.emit("delta", "value" to -SEEK_STEP_MS)
                    Player.COMMAND_SEEK_FORWARD -> Bridge.emit("delta", "value" to SEEK_STEP_MS)
                    else -> Bridge.emit("seek", "position" to position.coerceAtLeast(0))
                }
                return Futures.immediateVoidFuture()
            }
            override fun handleSetPlaybackParameters(parameters: PlaybackParameters): ListenableFuture<*> {
                Bridge.emit("rate", "value" to parameters.speed)
                return Futures.immediateVoidFuture()
            }
            override fun handleSetVolume(volume: Float): ListenableFuture<*> {
                Bridge.emit("volume", "value" to volume)
                return Futures.immediateVoidFuture()
            }
        }
        refreshSessionPosition = { routed.confirmPosition() }
        session = MediaSession.Builder(this, routed)
            .setSessionActivity(PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            .build()
        // JNI drives playback without a bound MediaController. Register eagerly
        // so notification and foreground promotion also work for the first UI play.
        addSession(session!!)
        Bridge.ready(this)
        handler.post(tick)
        Log.i("CarlitosAudio", "service created; player=${System.identityHashCode(player)}")
    }
    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    private fun installListener() {
        listener?.let { player.removeListener(it) }
        val currentGeneration = generation
        listener = object : Player.Listener {
            override fun onEvents(source: Player, events: Player.Events) {
                if (currentGeneration == generation && !applying) {
                    snapshot()
                    if (events.contains(Player.EVENT_IS_PLAYING_CHANGED) && !source.isPlaying &&
                        source.playbackState == Player.STATE_READY) confirmPause()
                }
            }
        }.also { player.addListener(it) }
    }
    fun command(command: JSONObject) {
        check(Looper.myLooper() == player.applicationLooper)
        applying = true
        try {
            val op = command.getString("op")
            if (command.has("token")) token = command.getLong("token")
            when (op) {
                "load" -> {
                    generation++
                    listener?.let { player.removeListener(it) }
                    player.stop()
                    player.clearMediaItems()
                    error = null
                    seekDone = false
                    loaded = true
                    val uri = command.getString("uri")
                    player.setPlaybackSpeed(command.getDouble("rate").toFloat())
                    player.skipSilenceEnabled = command.getBoolean("silence")
                    player.setMediaItem(MediaItem.Builder().setUri(uri).setMediaId("$generation:$uri")
                        .setMediaMetadata(MediaMetadata.Builder().setTitle(command.getString("title"))
                            .setIsPlayable(true).build()).build(), command.getLong("position"))
                    installListener()
                    player.prepare()
                    player.playWhenReady = command.getBoolean("playing")
                }
                "playing" -> player.playWhenReady = command.getBoolean("playing")
                "seek" -> { player.seekTo(command.getLong("position")); seekDone = true }
                "rate" -> player.setPlaybackSpeed(command.getDouble("rate").toFloat())
                "rate_step" -> player.setPlaybackSpeed((kotlin.math.round(player.playbackParameters.speed * 100 +
                    if (command.getBoolean("forward")) 5 else -5) / 100).coerceIn(0.5f, 3f))
                "silence" -> player.skipSilenceEnabled = command.getBoolean("enabled")
                "volume" -> player.volume = if (command.getBoolean("muted")) 0f else command.getDouble("volume").toFloat()
                "snapshot" -> if (barrierSince == null) barrierSince = SystemClock.elapsedRealtime()
                "stop" -> {
                    generation++
                    listener?.let { player.removeListener(it) }
                    listener = null
                    player.pause()
                    player.stop()
                    player.clearMediaItems()
                    loaded = false
                    error = null
                    seekDone = false
                }
            }
        } catch (failure: Exception) {
            error = failure.message ?: failure.toString()
        } finally {
            applying = false
            snapshot()
            // Rust may already await a newer command token while an earlier
            // pause acknowledgement is in transit. Confirm the latest paused
            // command too, after its audio-thread work has completed.
            if (!player.isPlaying && player.playbackState == Player.STATE_READY) confirmPause()
        }
    }
    private fun confirmPause() {
        val expectedGeneration = generation
        // playWhenReady changes optimistically on the main thread. Sample
        // again after the audio thread has actually stopped and updated its
        // source clock, without blocking input or discarding queued samples.
        player.createMessage { _, _ -> handler.post {
            // A queued speed/silence change may advance the command token
            // while the same recording remains paused. Publish its current
            // state under the current token instead of losing this final clock.
            if (!released && generation == expectedGeneration && !player.isPlaying) {
                refreshSessionPosition?.invoke()
                snapshot()
            }
        } }.send()
    }
    private fun snapshot() {
        if (released || applying) return
        val failure = player.playerError
        if (failure != null) error = Documents.accessError(failure) ?: "${failure.errorCodeName}: ${failure.message}"
        val ready = player.playbackState == Player.STATE_READY || player.playbackState == Player.STATE_ENDED
        // A provider may remain blocked while opening a recording. Acknowledge
        // the checkpoint within two seconds, as on desktop; Rust keeps the
        // last confirmed position while the player is still loading.
        val barrierReady = barrierSince?.let {
            ready || !loaded || error != null || SystemClock.elapsedRealtime() - it >= 2000
        } ?: false
        val phase = when {
            error != null -> "error"
            !loaded -> "empty"
            ready -> "ready"
            else -> "loading"
        }
        // Read current Media3 state, never data captured by an old load/seek callback.
        // Rust rejects snapshots whose command token was superseded in transit.
        val report = JSONObject().put("token", token).put("generation", generation)
            .put("position", player.currentPosition).put("playing", player.isPlaying)
            .put("rate", player.playbackParameters.speed).put("silence", player.skipSilenceEnabled)
            .put("volume", player.volume).put("duration", player.duration)
            .put("uri", player.currentMediaItem?.localConfiguration?.uri)
            .put("phase", phase).put("player", System.identityHashCode(player))
            .put("error", error ?: JSONObject.NULL)
        diagnostics = report.toString()
        Bridge.emit("playback", "token" to token, "position" to player.currentPosition.coerceAtLeast(0),
            "duration" to player.duration.takeIf { it >= 0 }, "playing" to
                (error == null && player.playWhenReady && player.playbackSuppressionReason == Player.PLAYBACK_SUPPRESSION_REASON_NONE),
            "phase" to phase, "seekable" to player.isCurrentMediaItemSeekable,
            "seek_done" to (seekDone && ready), "ended" to (player.playbackState == Player.STATE_ENDED),
            "barrier" to barrierReady, "error" to error,
            "rate" to player.playbackParameters.speed, "silence" to player.skipSilenceEnabled)
        if (ready || !loaded || error != null) seekDone = false
        if (barrierReady) barrierSince = null
        handler.removeCallbacks(tick)
        if (error == null && (player.isPlaying || player.isLoading || barrierSince != null)) {
            handler.postDelayed(tick, 250)
        }
    }
    override fun dump(fd: java.io.FileDescriptor, writer: java.io.PrintWriter, args: Array<out String>?) {
        writer.println("CarlitosAudio $diagnostics")
    }
    override fun onDestroy() {
        // Best effort only; regular checkpoints and pause do not depend on this.
        player.pause()
        snapshot()
        released = true
        handler.removeCallbacksAndMessages(null)
        if (Bridge.service === this) Bridge.service = null
        session?.release()
        session = null
        refreshSessionPosition = null
        player.release()
        Bridge.emit("service_stopped")
        Log.i("CarlitosAudio", "service destroyed")
        super.onDestroy()
    }
}
