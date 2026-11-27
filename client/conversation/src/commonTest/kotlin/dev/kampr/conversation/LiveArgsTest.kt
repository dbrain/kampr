package dev.kampr.conversation

import kotlin.test.Test
import kotlin.test.assertEquals

class LiveArgsTest {
    // The arguments arrive as the JSON the model is still writing; the summary is the node's key
    // order read from whatever has arrived so far.
    @Test
    fun aLiveCallIsSummarisedFromItsArgumentsSoFar() {
        val cases = listOf(
            "" to null,
            """{"comm""" to null,
            """{"command":"cargo te""" to "cargo te",
            """{"command":"cargo test -p kampr-node","description":"Run the node tests"}""" to "Run the node tests",
            """{"file_path":"/home/dbrain/dev/kampr/crates/kampr-node/src/lib.rs","old_string":"x"}""" to "/home/dbrain/dev/kampr/crates/kampr-node/src/lib.rs",
            """{"command":"echo \"hi\" && ls\nmore""" to "echo \"hi\" && ls",
            """{"todos":[]}""" to null,
        )
        for ((args, said) in cases) assertEquals(said, liveArgsSummary(args), args)
    }
}
