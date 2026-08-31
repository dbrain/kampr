package dev.kampr.shared.model

import dev.kampr.shared.wire.Style

class StyleTable {
    private val entries = mutableListOf(Style())

    val size: Int get() = entries.size

    // Ids are the node's and it interns them per socket, so the second connection's id 3 is
    // whatever pen it happened to meet third. Anything that resolves this table once and reads it
    // per cell has to be able to see that it moved, and the size cannot say so: a reconnect that
    // meets fewer pens than the last one leaves it exactly where it was.
    var version: Int = 0
        private set

    fun append(from: Int, styles: List<Style>) {
        while (entries.size < from) entries.add(Style())
        for ((offset, style) in styles.withIndex()) {
            val index = from + offset
            if (index < entries.size) entries[index] = style else entries.add(style)
        }
        version++
    }

    operator fun get(id: Int): Style = entries.getOrElse(id) { entries[0] }

    fun reset() {
        entries.clear()
        entries.add(Style())
        version++
    }
}
