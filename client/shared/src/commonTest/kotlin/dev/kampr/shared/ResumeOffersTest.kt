package dev.kampr.shared

import dev.kampr.shared.model.ResumeOffers
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ResumeRecord
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private const val RECORD = """{"id":"01k8/74877","label":"work","argv":["nvim","-S","/home/u/s.vim"],"cwd":"/home/u"}"""

private val NVIM = ResumeRecord("01k8/74877", "work", listOf("nvim", "-S", "/home/u/s.vim"), "/home/u")

private val HUB = NodeInfo(id = "01JHUB", name = "hub", kind = "local", online = true, native = true, resume = true)
private val PEER = NodeInfo(id = "01JPEER", name = "peer", kind = "peer", online = true, native = true, resume = true)
private val OLD = NodeInfo(id = "01JOLD", name = "old", kind = "peer", online = true)

// A 0.1.153 node keeps sessions and refuses the op, which would be an error on every connect.
private val KEEPS = NodeInfo(id = "01JKEEPS", name = "keeps", kind = "peer", online = true, native = true)

private fun managed(json: String) = Wire.decode(json) as ServerMsg.Managed

private class Offers {
    val sent = mutableListOf<ClientMsg.Manage>()
    val opened = mutableListOf<String?>()
    val refused = mutableListOf<ServerMsg.Managed>()
    val offers = ResumeOffers({ sent += it as ClientMsg.Manage }, { opened += it }, { refused += it })

    fun answer(node: String, records: String = "[$RECORD]") {
        val rid = sent.last { it.request == ManageOp.NativeResumable(node) }.rid
        offers.acked(managed("""{"t":"managed","op":"native.resumable","ok":true,"rid":"$rid","resumable":$records}"""))
    }
}

// docs/25's Resume after reboot: a node keeps what a boot left, and the herd asks each machine
// for it, shows it per machine, and runs or drops one record on a person's tap.
class ResumeOffersTest {
    // `claude.resumable` answers stored conversations under the same key; the command is what a
    // person reads before running one, so it has to survive the decode.
    @Test
    fun aNativeAnswerKeepsTheCommandAndItsLabel() {
        val ack = managed("""{"t":"managed","op":"native.resumable","ok":true,"rid":"r","resumable":[$RECORD]}""")
        assertEquals(listOf(NVIM), ack.resumeRecords)
        val claude = managed("""{"t":"managed","op":"claude.resumable","ok":true,"resumable":[{"id":"u","cwd":"/w"}]}""")
        assertNull(claude.resumeRecords)
    }

    @Test
    fun theOpsGoOutAsTheNodeReadsThem() {
        assertEquals(
            """{"t":"manage","op":"native.resumable","node":"01JHUB","rid":"x"}""",
            Wire.encode(ClientMsg.Manage(ManageOp.NativeResumable("01JHUB"), "x")),
        )
        assertEquals(
            """{"t":"manage","op":"native.resume","node":"01JHUB","id":"01k8/74877"}""",
            Wire.encode(ClientMsg.Manage(ManageOp.NativeResume("01JHUB", "01k8/74877"))),
        )
        assertEquals(
            """{"t":"manage","op":"native.resume.dismiss","node":"01JHUB","id":"01k8/74877"}""",
            Wire.encode(ClientMsg.Manage(ManageOp.NativeResumeDismiss("01JHUB", "01k8/74877"))),
        )
    }

    // Once per machine while the socket lasts: the herd changes with every status, and a node that
    // keeps no sessions is not asked at all. A machine that went away and came back has rebooted
    // as far as anyone here can tell, so it is asked again.
    @Test
    fun eachMachineThatKeepsSessionsIsAskedOnceUntilItOrTheSocketGoes() {
        val o = Offers()
        o.offers.herd(listOf(HUB, PEER, OLD, KEEPS))
        o.offers.herd(listOf(HUB, PEER, OLD, KEEPS))
        assertEquals(listOf(ManageOp.NativeResumable("01JHUB"), ManageOp.NativeResumable("01JPEER")), o.sent.map { it.request })

        o.offers.herd(listOf(HUB, PEER.copy(online = false, reachable = false)))
        o.offers.herd(listOf(HUB, PEER))
        assertEquals(ManageOp.NativeResumable("01JPEER"), o.sent.last().request)
        assertEquals(3, o.sent.size)

        o.offers.disconnected()
        o.offers.herd(listOf(HUB))
        assertEquals(4, o.sent.size)
    }

    @Test
    fun eachMachinesAnswerIsItsOwnAndARefusalShowsNothing() {
        val o = Offers()
        o.offers.herd(listOf(HUB, PEER))
        o.answer("01JHUB")
        val rid = o.sent.last { it.request == ManageOp.NativeResumable("01JPEER") }.rid
        o.offers.acked(managed("""{"t":"managed","op":"native.resumable","ok":false,"rid":"$rid","code":"bad_request","message":"unknown op"}"""))
        assertEquals(listOf(NVIM), o.offers.on("01JHUB"))
        assertEquals(emptyList(), o.offers.on("01JPEER"))
        assertEquals(emptyList(), o.refused, "a node without the op is not a failure anybody asked about")

        o.offers.disconnected()
        assertEquals(emptyList(), o.offers.on("01JHUB"))
    }

    @Test
    fun resumeRunsTheRecordAndOpensWhatItMade() {
        val o = Offers()
        o.offers.herd(listOf(HUB))
        o.answer("01JHUB")
        o.offers.resume("01JHUB", NVIM)
        assertEquals(ManageOp.NativeResume("01JHUB", "01k8/74877"), o.sent.last().request)
        assertEquals(emptyList(), o.offers.on("01JHUB"), "a record is spent the moment it is asked for")
        o.offers.acked(managed("""{"t":"managed","op":"native.resume","ok":true,"rid":"${o.sent.last().rid}","id":"01JHUB/n:s9"}"""))
        assertEquals(listOf<String?>("01JHUB/n:s9"), o.opened)
    }

    @Test
    fun aRefusedResumeIsSaid() {
        val o = Offers()
        o.offers.herd(listOf(HUB))
        o.answer("01JHUB")
        o.offers.resume("01JHUB", NVIM)
        o.offers.acked(
            managed("""{"t":"managed","op":"native.resume","ok":false,"rid":"${o.sent.last().rid}","code":"bad_request","message":"cannot be resumed"}"""),
        )
        assertEquals(emptyList(), o.opened)
        assertEquals(listOf("cannot be resumed"), o.refused.map { it.message })
    }

    @Test
    fun dismissDropsTheRecordWithoutRunningIt() {
        val o = Offers()
        o.offers.herd(listOf(HUB))
        o.answer("01JHUB")
        o.offers.dismiss("01JHUB", NVIM)
        assertEquals(ManageOp.NativeResumeDismiss("01JHUB", "01k8/74877"), o.sent.last().request)
        assertEquals(emptyList(), o.offers.on("01JHUB"))
        o.offers.acked(managed("""{"t":"managed","op":"native.resume.dismiss","ok":true,"rid":"${o.sent.last().rid}"}"""))
        assertEquals(emptyList(), o.opened)
        assertEquals(emptyList(), o.refused)
    }
}
