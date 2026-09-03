package dev.kampr.shared

import dev.kampr.shared.model.AnswerRefusal
import dev.kampr.shared.model.FleetRefused
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.cohorts
import dev.kampr.shared.model.fleetTargets
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.balanced
import dev.kampr.shared.model.matching
import dev.kampr.shared.model.recipients
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.FleetInfo
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Question
import dev.kampr.shared.wire.QuestionOption
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.fields
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The board's arithmetic, kept in step with `crates/kampr-client/tests/cohorts.rs`. The two clients
// must not disagree about which host needs somebody.
class FleetTest {
    private fun node(id: String, online: Boolean = true, reachable: Boolean? = null) =
        NodeInfo(id = id, name = id, kind = "peer", online = online, reachable = reachable)

    private fun fleet(
        cohort: String,
        state: String,
        question: Question? = null,
        exitCode: Int? = if (state == "exited") 0 else null,
        signal: Int? = null,
        command: String = "pacman -Syu",
    ) = FleetInfo(
        cohort = cohort,
        command = command,
        state = state,
        question = question,
        exitCode = exitCode,
        signal = signal,
        quietSeconds = if (state == "quiet") 45 else null,
        startedUnix = 1_700_000_000,
    )

    private fun pane(nodeId: String, id: String, fleet: FleetInfo? = null) =
        PaneInfo(id = "$nodeId/$id", nodeId = nodeId, fleet = fleet)

    private fun herd(vararg panes: PaneInfo) =
        Herd(nodes = listOf(node("n1"), node("n2"), node("n3")), panes = panes.toList(), known = true)

    private fun confirm(prompt: String) = Question(
        prompt = prompt,
        shape = "confirm",
        options = listOf(QuestionOption("y", "Y"), QuestionOption("n", "n")),
        defaultKey = "y",
    )

    @Test
    fun aFleetRunIsNeverListedBesideTheOperatorsOwnPanes() {
        // The whole reason a fleet run is not a herdr pane: it must not clutter the desk it runs on.
        val h = herd(pane("n1", "w1:p1"), pane("n1", "fleet:01A", fleet("upgrade", "running")))
        assertContentEquals(listOf("n1/w1:p1"), h.groups().flatMap { g -> g.panes.map { it.id } })
        assertEquals(1, h.cohorts().size, "but it is on the board")
    }

    @Test
    fun oneCommandAcrossTwoHostsIsOneCohort() {
        val h = herd(
            pane("n1", "fleet:01A", fleet("upgrade", "running")),
            pane("n2", "fleet:01B", fleet("upgrade", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n1", "fleet:01C", fleet("reboot", "running")),
        )
        assertEquals(2, h.cohorts().size)
        val upgrade = h.cohorts().first { it.id == "upgrade" }
        assertEquals(2, upgrade.panes.size)
        assertEquals(1, upgrade.waiting)
        assertEquals(1, upgrade.running)
        assertFalse(upgrade.finished)
    }

    @Test
    fun theBoardPutsWhatNeedsSomebodyAtTheTopAndFailuresAboveSuccesses() {
        val h = herd(
            pane("n1", "fleet:01A", fleet("c", "exited")),
            pane("n1", "fleet:01B", fleet("c", "quiet")),
            pane("n1", "fleet:01C", fleet("c", "exited", exitCode = 1)),
            pane("n1", "fleet:01D", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n1", "fleet:01E", fleet("c", "running")),
        )
        val states = h.cohorts()[0].panes.map { it.fleet!!.state }
        assertEquals("waiting", states[0], "the point of the board is at the top")
        assertEquals("running", states[1])
        assertEquals("quiet", states[2])
        assertEquals(1, h.cohorts()[0].panes[3].fleet!!.exitCode, "the failure sorts above the success")
        assertEquals(0, h.cohorts()[0].panes[4].fleet!!.exitCode)
    }

    @Test
    fun aRunTheKernelKilledIsFinishedAndIsNotASuccess() {
        // A signal has no exit code, and rounding it to zero would report a death as a clean
        // upgrade.
        val h = herd(pane("n1", "fleet:01A", fleet("c", "exited", exitCode = null, signal = 15)))
        val cohort = h.cohorts()[0]
        assertTrue(cohort.finished)
        assertEquals(0, cohort.succeeded)
        assertEquals(1, cohort.failed)
    }

    @Test
    fun aQuietHostIsCountedApartFromOneThatIsAsking() {
        // Probes #331/#332: a host whose state cannot be read is not a host with a question, and a
        // board that added them together would send somebody to a machine that is only slow.
        val h = herd(
            pane("n1", "fleet:01A", fleet("c", "quiet")),
            pane("n2", "fleet:01B", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
        )
        assertEquals(1, h.cohorts()[0].waiting)
        assertEquals(1, h.cohorts()[0].quiet)
    }

    @Test
    fun oneAnswerReachesEveryHostAskingByteIdentically() {
        val h = herd(
            pane("n1", "fleet:01A", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n2", "fleet:01B", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
        )
        val match = h.matching("n1/fleet:01A").getOrThrow()
        assertEquals(2, match.reach)
        assertTrue(match.differing.isEmpty())
        assertContentEquals(listOf("n1/fleet:01A", "n2/fleet:01B"), match.recipients())
    }

    @Test
    fun aHostAskingSomethingElseIsNamedAndNotAnswered() {
        // The silent third of the fleet is what bites you: it must be visible, and it must not be
        // sent an answer to a question it did not ask.
        val h = herd(
            pane("n1", "fleet:01A", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n2", "fleet:01B", fleet("c", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n3", "fleet:01C", fleet("c", "waiting", confirm("Remove kdelibs4support-git? [y/N]"))),
        )
        val match = h.matching("n1/fleet:01A").getOrThrow()
        assertEquals(2, match.reach)
        assertEquals(listOf("n3/fleet:01C"), match.differing.map { it.id })
        assertFalse("n3/fleet:01C" in match.recipients())
    }

    @Test
    fun aDifferentCohortIsNeverSweptInHoweverAlikeItsQuestion() {
        val h = herd(
            pane("n1", "fleet:01A", fleet("upgrade", "waiting", confirm("Proceed? [Y/n]"))),
            pane("n2", "fleet:01B", fleet("other", "waiting", confirm("Proceed? [Y/n]"))),
        )
        assertEquals(1, h.matching("n1/fleet:01A").getOrThrow().reach)
    }

    @Test
    fun aPasswordIsAnsweredOneHostAtATime() {
        // Every password prompt in the world says "Password:", so a text match is no evidence that
        // two hosts want the same secret — and being wrong means handing it to the one that did not.
        val secret = Question(prompt = "Password:", shape = "secret")
        val h = herd(
            pane("n1", "fleet:01A", fleet("c", "waiting", secret)),
            pane("n2", "fleet:01B", fleet("c", "waiting", secret)),
        )
        val refusal = h.matching("n1/fleet:01A").exceptionOrNull()
        assertEquals(AnswerRefusal.Secret, (refusal as FleetRefused).refusal)
    }

    @Test
    fun aFullScreenProgramOffersNoButtonsAndIsNotASecret() {
        // Probe #340: `vim` and `less` turn ECHO off exactly as a password prompt does, and the
        // board must offer neither buttons nor a password box for one.
        val screen = Question(prompt = "~", shape = "screen", options = listOf(QuestionOption("y", "Y")))
        assertTrue(screen.ownsTheScreen)
        assertFalse(screen.isSecret)
        assertTrue(screen.answerable.isEmpty())
    }

    @Test
    fun anInferredQuestionArrivesLabelledAndAnOrdinaryOneDoesNot() {
        // Probe #341: under `sudo` the node cannot read the job and termios describes the relay, so
        // the screen speaks instead — and a client must be able to tell the two apart.
        val measured = Wire.json.decodeFromString(
            Question.serializer(),
            """{"prompt":"Proceed? [Y/n]","shape":"confirm"}""",
        )
        val guessed = Wire.json.decodeFromString(
            Question.serializer(),
            """{"prompt":"Proceed? [Y/n]","shape":"confirm","inferred":true}""",
        )
        assertFalse(measured.inferred)
        assertTrue(guessed.inferred)
    }

    @Test
    fun aHostWhoseHerdrIsDownIsStillRunOn() {
        // `online` is herdr's health, and a fleet run does not need herdr. A machine sitting right
        // there was being skipped for a reason that had nothing to do with running the command.
        assertEquals(1, fleetTargets(listOf(node("a", online = false, reachable = true))).size)
        assertEquals(0, fleetTargets(listOf(node("a", online = true, reachable = false))).size)
    }

    @Test
    fun anOlderNodeThatDoesNotSayFallsBackToOnline() {
        // Additive: a node from before the field behaves exactly as it did.
        assertEquals(1, fleetTargets(listOf(node("a", online = true, reachable = null))).size)
        assertEquals(0, fleetTargets(listOf(node("a", online = false, reachable = null))).size)
    }

    // **The wire's two shapes, and why only one of them is sent now.** `command` is the line the
    // operator typed, for the host's own shell; `args` is the argv `fleet.run` has always meant and
    // is what clients built before this send. Sending both would be a second answer for the node to
    // disagree with, and sending an argv now would put a `|` back in `find`'s hands.
    @Test
    fun aFleetRunCarriesTheLineTheOperatorTypedAndNoArgv() {
        val line = """find . -name "*.rs" | wc -l"""
        val fields = ManageOp.FleetRun(node = "n1", cohort = "c1", command = line).fields()
        assertEquals(line, (fields["command"] as JsonPrimitive).content)
        assertNull(fields["args"], "an argv beside the line is a second answer to disagree with")
    }

    // The only thing checked before a line reaches every machine in the herd. Everything else is
    // the host's own shell's, on purpose — `&&`, `|`, `;`, globs and quotes all mean there what
    // they mean in the operator's terminal. Kept in step with `kampr_client::fleet::balanced`.
    @Test
    fun onlyAnUnclosedQuoteIsRefusedAndEverythingElseIsTheShellsToRead() {
        assertTrue(balanced("""find . -name "*.rs" | wc -l"""))
        assertTrue(balanced("pacman -Syu && reboot"))
        assertTrue(balanced("""git commit -m 'a message'"""))
        assertFalse(balanced("""sh -c "echo oops"""), "an unclosed quote is refused, not guessed at")
        // A checker cruder than the shell it stands in front of would refuse a run that works.
        assertTrue(balanced("""echo \""""), "an escaped quote is not an unclosed one")
        assertTrue(balanced("""echo "a \" b""""))
        assertTrue(balanced("""echo "don't""""))
        assertTrue(balanced("""echo 'a\'"""), "inside single quotes a backslash is a backslash")
    }

    @Test
    fun aPaneEntryWithoutAFleetBlockDecodesExactlyAsItAlwaysDid() {
        // The wire is additive: an older node sends no `fleet`, and a pane without one is an
        // ordinary pane rather than a broken fleet run.
        val json = """{"id":"n1/w1:p1","node_id":"n1","rows":30}"""
        val pane = Wire.json.decodeFromString(PaneInfo.serializer(), json)
        assertNull(pane.fleet)
    }

    @Test
    fun aFleetBlockDecodesWithItsQuestionAndItsExitStatus() {
        val json = """
            {"id":"n1/fleet:01A","node_id":"n1","rows":30,
             "fleet":{"cohort":"c","command":"pacman -Syu","state":"waiting",
                      "question":{"prompt":":: Proceed with installation? [Y/n]","shape":"confirm",
                                  "options":[{"key":"y","label":"Y"},{"key":"n","label":"n"}],
                                  "default_key":"y"},
                      "blind":false,"started_unix":1700000000}}
        """.trimIndent()
        val pane = Wire.json.decodeFromString(PaneInfo.serializer(), json)
        val fleet = pane.fleet!!
        assertEquals("c", fleet.cohort)
        assertTrue(fleet.isWaiting)
        val question = fleet.question!!
        assertEquals("y", question.defaultKey)
        assertEquals(2, question.answerable.size)
        assertFalse(fleet.succeeded)
    }

    // The frame the node pushes unasked, into the state the sheet renders. Without this the
    // decode could be wrong in any way at all and only a running node would say so.
    @Test
    fun theFleetBookDecodesOffTheWireAndLandsInTheStore() {
        val frame = """
            {"t":"fleet.book",
             "recent":[{"id":"b2","args":["pacman","-Syu"],"at":1774000000}],
             "saved":[{"id":"b1","args":["kampr","update"],"cwd":"/srv",
                       "label":"update everything","at":1774000001}]}
        """.trimIndent()
        val store = KamprStore()
        store.accept(Wire.decode(frame)!!)
        val book = store.book.value
        assertEquals(listOf("pacman", "-Syu"), book.recent.single().args)
        assertEquals("pacman -Syu", book.recent.single().command)
        assertNull(book.recent.single().label)
        assertEquals("update everything", book.saved.single().label)
        assertEquals("/srv", book.saved.single().cwd)
        assertEquals("b1", book.saved.single().id)
    }

    // A delete is an absence, and a merge cannot express one — so the book is replaced whole
    // rather than merged the way `prefs` is.
    @Test
    fun aLaterBookReplacesTheOneBeforeItRatherThanMergingWithIt() {
        val store = KamprStore()
        store.accept(Wire.decode("""{"t":"fleet.book","recent":[{"id":"b2","args":["uptime"]}]}""")!!)
        store.accept(Wire.decode("""{"t":"fleet.book","recent":[],"saved":[]}""")!!)
        assertEquals(emptyList(), store.book.value.recent)
    }

    // The two ops carry `entry` and never `at`: `at` is what the node routes on, and a book entry
    // names no host — one sent there would go down a mesh link looking for the node that owns it.
    @Test
    fun aBookOpAddressesAnEntryAndNeverARoutedTarget() {
        val save = Wire.encode(ClientMsg.Manage(ManageOp.FleetSave(entry = "b1", label = "load")))
        assertEquals("""{"t":"manage","op":"fleet.save","entry":"b1","label":"load"}""", save)
        assertEquals(
            """{"t":"manage","op":"fleet.drop","entry":"b1"}""",
            Wire.encode(ClientMsg.Manage(ManageOp.FleetDrop("b1"))),
        )
    }

    @Test
    fun anUnknownFleetStateIsIgnoredRatherThanBreakingTheDecode() {
        // The wire rule: a value this build has never heard of must not take the message down.
        val json = """
            {"id":"n1/fleet:01A","node_id":"n1","rows":30,
             "fleet":{"cohort":"c","state":"something-new-later","started_unix":1}}
        """.trimIndent()
        val pane = Wire.json.decodeFromString(PaneInfo.serializer(), json)
        val fleet = pane.fleet!!
        assertEquals("something-new-later", fleet.state)
        assertFalse(fleet.isWaiting)
        assertFalse(fleet.isFinished)
    }
}
