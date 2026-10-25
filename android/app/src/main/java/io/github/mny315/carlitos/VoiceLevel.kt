package io.github.mny315.carlitos

import android.os.SystemClock
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.ForwardingAudioSink
import java.nio.ByteBuffer
import kotlin.math.sqrt
import kotlin.math.exp
import kotlin.math.roundToInt

/** Four broad voice bands; bounded, read-only PCM analysis at 12.5 Hz. */
@UnstableApi
internal object VoiceLevel {
    @Volatile var enabled = false
        set(value) { field = value; level = 0f; packedBands = 0L; updated = 0L }
    @Volatile var foreground = false
        set(value) { field = value; level = 0f; packedBands = 0L; updated = 0L }
    @Volatile private var level = 0f
    @Volatile private var packedBands = 0L
    @Volatile private var updated = 0L
    @Volatile var sampleCount = 0L
        private set
    // The audio thread owns the format and sampling clock.
    private var encoding = C.ENCODING_INVALID
    private var channels = 0
    private var sampleRate = 0
    private var stride = 1
    private val alpha = FloatArray(4)
    private val states = FloatArray(8)
    private val powers = DoubleArray(4)
    private val cutoffs = floatArrayOf(60f, 240f, 700f, 1800f)
    private val gains = floatArrayOf(1.35f, 1.9f, 2.3f, 1.45f)
    private var lastSample = 0L

    fun configure(format: Format) {
        encoding = format.pcmEncoding
        channels = format.channelCount
        sampleRate = format.sampleRate.coerceAtLeast(0)
        stride = (sampleRate / 12000).coerceAtLeast(1)
        val rate = sampleRate.toDouble() / stride
        for (i in 0..3) alpha[i] = if (rate > 0) (1 - exp(-2 * Math.PI * cutoffs[i] / rate)).toFloat() else 0f
        lastSample = 0L
        level = 0f
        packedBands = 0L
        updated = 0L
    }
    fun observe(buffer: ByteBuffer) {
        // Do not read the clock, PCM, allocate, or schedule work in background.
        if (!enabled || !foreground) return
        val now = SystemClock.uptimeMillis()
        if (now - lastSample < 80) return
        val bytes = when (encoding) {
            C.ENCODING_PCM_16BIT -> 2
            C.ENCODING_PCM_FLOAT, C.ENCODING_PCM_32BIT -> 4
            C.ENCODING_PCM_24BIT -> 3
            C.ENCODING_PCM_8BIT -> 1
            else -> return // Encoded/offloaded formats need no decoder changes.
        }
        if (channels <= 0 || sampleRate == 0) return
        val frameSize = bytes * channels
        val frames = buffer.remaining() / frameSize
        val observedChannels = channels.coerceAtMost(2)
        val count = (frames / stride).coerceAtMost(512 / observedChannels)
        if (count == 0) return
        lastSample = now
        val start = buffer.position()
        states.fill(0f)
        powers.fill(0.0)
        var inputPower = 0.0
        for (frame in 0 until count) {
            for (channel in 0 until observedChannels) {
                // Average before decimation to limit aliasing. Each channel
                // has separate filters, including opposite-phase stereo.
                var value = 0f
                for (offset in 0 until stride) {
                    val at = start + (frame * stride + offset) * frameSize + channel * bytes
                    value += pcmSample(buffer, at, bytes)
                }
                value /= stride
                val state = channel * 4
                states[state] += alpha[0] * (value - states[state])
                value -= states[state]
                inputPower += value * value
                for (i in 1..3) states[state + i] += alpha[i] * (value - states[state + i])
                val bass = states[state + 1]
                val body = states[state + 2] - bass
                val clarity = states[state + 3] - states[state + 2]
                val air = value - states[state + 3]
                powers[0] += bass * bass
                powers[1] += body * body
                powers[2] += clarity * clarity
                powers[3] += air * air
            }
        }
        val rms = sqrt(inputPower / (count * observedChannels)).toFloat()
        level = ((sqrt(rms) - .035f) * 2f).coerceIn(0f, 1f)
        var packed = 0L
        if (inputPower > 1e-7) for (i in 0..3) {
            val band = level * (sqrt(powers[i] / inputPower).toFloat() * gains[i]).coerceIn(0f, 1f)
            packed = packed or ((band * 255).roundToInt().toLong() shl (i * 8))
        }
        packedBands = packed
        updated = now
        sampleCount++
    }
    fun read(): Float = if (enabled && foreground && updated != 0L &&
        SystemClock.uptimeMillis() - updated <= 200) level else 0f
    fun readBands(): Long = if (enabled && foreground && updated != 0L &&
        SystemClock.uptimeMillis() - updated <= 200) packedBands else 0L

    private fun pcmSample(buffer: ByteBuffer, offset: Int, bytes: Int): Float {
        // Absolute little-endian reads preserve position, limit and order.
        var bits = buffer.get(offset).toInt() and 255
        for (byte in 1 until bytes) bits = bits or ((buffer.get(offset + byte).toInt() and 255) shl (byte * 8))
        val sample = when (encoding) {
            C.ENCODING_PCM_FLOAT -> Float.fromBits(bits)
            C.ENCODING_PCM_8BIT -> (bits - 128) / 128f
            C.ENCODING_PCM_16BIT -> bits.toShort() / 32768f
            C.ENCODING_PCM_24BIT -> (bits shl 8) / 2147483648f
            else -> bits / 2147483648f
        }
        return if (sample.isFinite()) sample.coerceIn(-1f, 1f) else 0f
    }
}

/** Observe the existing decoded buffer; the real sink retains all ownership. */
@UnstableApi
internal class VoiceLevelSink(sink: AudioSink) : ForwardingAudioSink(sink) {
    override fun configure(format: Format, specifiedBufferSize: Int, outputChannels: IntArray?) {
        super.configure(format, specifiedBufferSize, outputChannels)
        VoiceLevel.configure(format)
    }
    override fun handleBuffer(buffer: ByteBuffer, presentationTimeUs: Long, encodedAccessUnitCount: Int): Boolean {
        VoiceLevel.observe(buffer)
        return super.handleBuffer(buffer, presentationTimeUs, encodedAccessUnitCount)
    }
}
