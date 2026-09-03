package dev.kampr.shared

import dev.kampr.shared.model.secretish
import dev.kampr.shared.wire.Wire
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlin.test.fail

// `crates/kampr-node/tests/it/fleet_book.rs` asserts the node reads this exact file. The node uses
// the rule to decide what it writes to disk by itself; this half uses it to warn the operator
// before they save one on purpose. A client warning about a different set than the node declines
// is worse than either alone, so neither side owns the fixture.
private const val FIXTURE = "crates/kampr-node/tests/fixtures/secretish.json"

class FleetSecretTest {
    private val fixture: JsonObject by lazy {
        var dir = File(".").absoluteFile
        repeat(5) {
            val candidate = File(dir, FIXTURE)
            if (candidate.isFile) {
                return@lazy Wire.json.parseToJsonElement(candidate.readText()) as JsonObject
            }
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not find $FIXTURE from ${File(".").absolutePath}")
    }

    private fun argv(element: Any?): List<String> =
        (element as JsonArray).map { (it as JsonPrimitive).content }

    private fun section(name: String) = fixture[name] as JsonObject

    // Every case twice: as the argv the fixture writes, and as that argv joined into one string.
    // A fleet run is a command line now, and a book entry holds that line as its single argument,
    // so a rule that only read argv would warn about `TOKEN=abc ./deploy` and say nothing about
    // the identical line the operator typed.
    private fun bothWays(element: Any?): List<List<String>> {
        val words = argv(element)
        return listOf(words, listOf(words.joinToString(" ")))
    }

    @Test
    fun everyShapeTheNodeWillNotWriteDownIsOneThisClientWarnsAbout() {
        val caught = section("caught")
        assertTrue(caught.size >= 12, "the caught set must cover every shape the rule claims")
        for ((name, case) in caught) {
            val entry = case as JsonObject
            assertEquals(
                (entry["why"] as JsonPrimitive).contentOrNull,
                secretish(argv(entry["args"])),
                name,
            )
            // The joined shape asserts it still fires, not which word said so: read as one line,
            // `curl --oauth2-bearer eyJ` is caught by the bearer marker rather than by the flag,
            // and naming a different word is not the same as missing the credential.
            val line = listOf(argv(entry["args"]).joinToString(" "))
            assertTrue(secretish(line) != null, "$name typed as one line went unnoticed: $line")
        }
    }

    // The honest half. These carry a secret and are NOT caught — asserted so the blind spots are
    // written down and tested rather than claimed, and so nobody presents this rule to the
    // operator as a guarantee.
    @Test
    fun theShapesThisRuleCannotSeeAreNamedRatherThanClaimedAway() {
        for ((name, case) in section("missed")) {
            for (shape in bothWays(case)) {
                assertNull(secretish(shape), "$name is a documented blind spot; update the fixture")
            }
        }
    }

    @Test
    fun anOrdinaryCommandDoesNotCryWolf() {
        for ((name, case) in section("clean")) {
            for (shape in bothWays(case)) {
                assertNull(secretish(shape), "$name is not a secret, and a rule nobody believes is no rule")
            }
        }
    }
}
