package io.github.mny315.carlitos

import android.content.Context
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.audio.DefaultAudioTrackBufferSizeProvider

@UnstableApi
internal class AudioRenderers(context: Context) : DefaultRenderersFactory(context) {
    override fun buildAudioSink(context: Context, enableFloatOutput: Boolean,
        enableAudioTrackPlaybackParams: Boolean): AudioSink = DefaultAudioSink.Builder(context)
        .setEnableFloatOutput(enableFloatOutput)
        .setEnableAudioTrackPlaybackParams(enableAudioTrackPlaybackParams)
        // Bound the speech output queue while retaining AudioTrack's required
        // minimum for each device/format and Media3's normal output handling.
        .setAudioTrackBufferSizeProvider(DefaultAudioTrackBufferSizeProvider.Builder()
            .setPcmBufferMultiplicationFactor(1)
            .setMinPcmBufferDurationUs(80_000)
            .setMaxPcmBufferDurationUs(80_000)
            .build())
        .build()
}
