package dev.kampr.conversation

import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.unit.IntSize

sealed interface Thumbnail {
    data object Idle : Thumbnail
    data object Fetching : Thumbnail
    data class Shown(val image: ImageBitmap) : Thumbnail
    data class Failed(val reason: String) : Thumbnail
}

// Wide enough for a tile on a 3x screen. A 1080×2400 screenshot is ~10 MB decoded and ~0.4 MB at
// this size, which is what lets a gallery keep every tile it has drawn.
internal const val THUMBNAIL_PX = 384

internal fun ImageBitmap.within(px: Int): ImageBitmap {
    val longest = maxOf(width, height)
    if (longest <= px) return this
    val size = IntSize(
        (width.toLong() * px / longest).toInt().coerceAtLeast(1),
        (height.toLong() * px / longest).toInt().coerceAtLeast(1),
    )
    val out = ImageBitmap(size.width, size.height)
    Canvas(out).drawImageRect(this, dstSize = size, paint = Paint().apply { filterQuality = FilterQuality.Medium })
    return out
}
