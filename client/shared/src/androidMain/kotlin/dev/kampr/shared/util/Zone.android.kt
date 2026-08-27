package dev.kampr.shared.util

import java.util.TimeZone

actual fun localOffsetMillis(atMillis: Double): Double =
    TimeZone.getDefault().getOffset(atMillis.toLong()).toDouble()
