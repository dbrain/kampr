package dev.kampr.shared.model

import dev.kampr.shared.wire.Style

class StyleTable {
    private val entries = mutableListOf(Style())

    val size: Int get() = entries.size

    fun append(from: Int, styles: List<Style>) {
        while (entries.size < from) entries.add(Style())
        for ((offset, style) in styles.withIndex()) {
            val index = from + offset
            if (index < entries.size) entries[index] = style else entries.add(style)
        }
    }

    operator fun get(id: Int): Style = entries.getOrElse(id) { entries[0] }

    fun reset() {
        entries.clear()
        entries.add(Style())
    }
}
