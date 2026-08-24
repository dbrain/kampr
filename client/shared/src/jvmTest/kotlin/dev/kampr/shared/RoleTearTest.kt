package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ServerMsg
import java.util.concurrent.CountDownLatch
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals

// A role change is one fact — the role, and what to tell the operator about it — and the store
// wrote it as two independent snapshot states, one statement apart. A reader watching for the
// change lands between them, sees the role it was waiting for and a note that is still null, and
// reports that a demotion was never surfaced. On CI this is the whole of `RoleChangeTest`'s
// intermittent failure; in the app it is a notice that renders a frame late, until it is the
// frame someone reads.
class RoleTearTest {
    @Test
    fun aRoleAndTheNoticeThatExplainsItAreOneChangeRatherThanTwo() {
        var torn = 0
        repeat(ROUNDS) {
            val store = KamprStore()
            val ready = CountDownLatch(1)
            var seen: String? = null
            var found = false
            val reader = thread {
                ready.countDown()
                while (!store.readOnly) Thread.onSpinWait()
                seen = store.roleNote
                found = true
            }
            ready.await()
            store.accept(ServerMsg.RoleChanged("readonly"))
            reader.join(5_000)
            if (found && seen == null) torn++
        }
        assertEquals(
            0,
            torn,
            "a reader that watched the role move saw no notice with it, $torn times in $ROUNDS",
        )
    }
}

private const val ROUNDS = 20_000
