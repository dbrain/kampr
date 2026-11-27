package dev.kampr.conversation

import dev.kampr.shared.wire.ContextUse
import kotlin.test.Test
import kotlin.test.assertEquals

class ContextWordTest {
    @Test
    fun aKnownWindowIsSaidAndAnUnknownOneIsLeftOut() {
        assertEquals("context 1k/200k", contextWord(ContextUse(used = 1200, total = 200_000)))
        assertEquals("context 1k", contextWord(ContextUse(used = 1200, total = 0)))
    }
}
