package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Style
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val GREETING =
    """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"cc","role":"full"}"""

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

private fun styles(from: Int, vararg colours: Int): String {
    val pens = colours.joinToString(",") { """{"bg":{"k":"i","v":$it}}""" }
    return """{"t":"styles","from":$from,"styles":[$pens]}"""
}

// Style ids are the node's, they are interned per socket, and the node says so: "ids are stable
// for the life of a connection". A phone that goes to the background and comes back gets a new
// socket and a new table, whose id 3 is whatever pen that connection happened to meet third.
class StylePaletteTest {
    @Test
    fun aPenFromTheConnectionBeforeDoesNotAnswerForAnIdThisOneHasNotSent() {
        val store = KamprStore()
        store.take(GREETING)
        store.take(styles(1, 1, 2, 3, 4, 5))
        assertEquals(ColorSpec.Indexed(3), store.styles[3].bg)

        store.take(GREETING)
        store.take(styles(1, 7))
        assertEquals(
            Style(),
            store.styles[3],
            "id 3 belongs to whichever pen this connection interns third, and it has not sent one",
        )
        assertEquals(ColorSpec.Indexed(7), store.styles[1].bg, "and the pens it did send are its own")
    }
}
