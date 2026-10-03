package io.github.mny315.carlitos

import android.content.res.AssetFileDescriptor
import android.media.MediaCodecList
import android.media.MediaExtractor
import android.media.MediaFormat
import java.nio.ByteBuffer

/** Inspect the container and an encoded sample; playback owns audio decoding. */
object AudioProbe {
    private val codecs by lazy { MediaCodecList(MediaCodecList.REGULAR_CODECS) }

    fun inspect(asset: AssetFileDescriptor): Long? {
        val extractor = MediaExtractor()
        try {
            if (asset.declaredLength >= 0) extractor.setDataSource(asset.fileDescriptor, asset.startOffset, asset.declaredLength)
            else {
                check(asset.startOffset == 0L) { "Unknown length of document slice" }
                extractor.setDataSource(asset.fileDescriptor)
            }
            val index = (0 until extractor.trackCount).firstOrNull {
                extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: error("Unsupported or damaged audio document: no audio track")
            val format = extractor.getTrackFormat(index)
            val mime = format.getString(MediaFormat.KEY_MIME)!!
            check(mime == "audio/raw" || codecs.findDecoderForFormat(format) != null) {
                "Unsupported audio format on this device: $mime"
            }
            extractor.selectTrack(index)
            // Some extractors expose decoded PCM (including FLAC). A valid
            // multichannel frame can exceed the usual compressed-packet size.
            val sampleSize = if (android.os.Build.VERSION.SDK_INT >= 28) extractor.sampleSize
                else if (format.containsKey(MediaFormat.KEY_MAX_INPUT_SIZE)) format.getInteger(MediaFormat.KEY_MAX_INPUT_SIZE).toLong()
                else 0L
            val capacity = sampleSize.coerceAtLeast(256 * 1024)
            check(capacity <= 8 * 1024 * 1024) { "Audio sample is too large" }
            check(extractor.readSampleData(ByteBuffer.allocate(capacity.toInt()), 0) > 0) {
                "Empty or damaged audio document"
            }
            // Match MediaMetadataRetriever's nearest-millisecond rounding so
            // previously imported durations still match during source moves.
            return if (format.containsKey(MediaFormat.KEY_DURATION)) {
                val micros = format.getLong(MediaFormat.KEY_DURATION)
                (micros / 1000 + if (micros % 1000 >= 500) 1 else 0).takeIf { it > 0 }
            } else null
        } finally { extractor.release() }
    }
}
