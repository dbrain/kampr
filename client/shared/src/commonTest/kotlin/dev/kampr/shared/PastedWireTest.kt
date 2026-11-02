package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PendingFile
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JNODE/w1:p1"

class PastedWireTest {
    @Test
    fun aTypedPasteSaysNothingNewAndAQuietOneSaysSo() {
        assertEquals(
            """{"t":"paste","pane":"$PANE","b64":"aGk=","name":"a"}""",
            Wire.encode(ClientMsg.Paste(PANE, "aGk=", "a")),
        )
        assertEquals(
            """{"t":"paste","pane":"$PANE","b64":"aGk=","name":"a","typed":false,"id":"c1"}""",
            Wire.encode(ClientMsg.Paste(PANE, "aGk=", "a", typed = false, id = "c1")),
        )
    }

    @Test
    fun theAnswerReadiesTheFileItNamesAndNoOther() {
        val store = KamprStore()
        val files = store.pane(PANE).files
        val mine = files.add("shot.png", byteArrayOf(1))
        val other = files.add("log.txt", byteArrayOf(2))

        store.accept(
            Wire.decode("""{"t":"pasted","pane":"$PANE","id":"${mine.id}","path":"/p/shot-1.png","text":"/p/shot-1.png "}""")!!,
        )
        store.accept(Wire.decode("""{"t":"pasted","pane":"$PANE","id":"${other.id}","refused":"too big"}""")!!)
        store.accept(Wire.decode("""{"t":"pasted","pane":"$PANE","id":"someone-elses","path":"/p/x"}""")!!)

        assertEquals(
            listOf(PendingFile.State.Ready("/p/shot-1.png", "/p/shot-1.png "), PendingFile.State.Refused("too big")),
            files.all.map { it.state },
        )
    }

    @Test
    fun aPaneErrorWithNoIdRefusesOnlyWhatIsStillOnItsWay() {
        val store = KamprStore()
        val files = store.pane(PANE).files
        val ready = files.add("a", byteArrayOf(1))
        store.accept(ServerMsg.Pasted(PANE, ready.id, path = "/p/a", text = "/p/a "))
        files.add("b", byteArrayOf(2))

        store.accept(ServerMsg.Failure("node_offline", "the node serving this pane left the herd", PANE))

        assertEquals(
            listOf(PendingFile.State.Ready("/p/a", "/p/a "), PendingFile.State.Refused("the node serving this pane left the herd")),
            files.all.map { it.state },
        )
    }
}
