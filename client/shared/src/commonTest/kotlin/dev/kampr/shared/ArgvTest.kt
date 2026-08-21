package dev.kampr.shared

import dev.kampr.shared.util.bypassesSafety
import dev.kampr.shared.util.commandLine
import dev.kampr.shared.util.parseArgs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// `agent.start` takes an argv array, not a command string: the node forwards it to herdr as one,
// and herdr execs it. So a typed line has to become the same argv a shell would have made of it —
// probe #95 confirms `-- --dangerously-skip-permissions` arrives as argv[1].
class ArgvTest {
    @Test
    fun aTypedLineBecomesTheArgvAShellWouldHaveMade() {
        val cases = listOf(
            "" to emptyList(),
            "   " to emptyList(),
            "--dangerously-skip-permissions" to listOf("--dangerously-skip-permissions"),
            "  --model   opus  " to listOf("--model", "opus"),
            "--append-system-prompt \"be terse\"" to listOf("--append-system-prompt", "be terse"),
            "--append-system-prompt 'be terse'" to listOf("--append-system-prompt", "be terse"),
            """--x "a 'b' c"""" to listOf("--x", "a 'b' c"),
            "--x=y" to listOf("--x=y"),
            // An unclosed quote is a typo, not a reason to drop the rest of the line.
            "--x \"unclosed" to listOf("--x", "unclosed"),
        )
        for ((typed, expected) in cases) assertEquals(expected, parseArgs(typed), "typed: $typed")
    }

    // What the sheet prints back, so the operator reads the launch rather than trusting it.
    @Test
    fun theLaunchIsPrintedBackAsOneLine() {
        assertEquals("claude", commandLine("claude", emptyList()))
        assertEquals(
            "claude --dangerously-skip-permissions",
            commandLine("claude", listOf("--dangerously-skip-permissions")),
        )
        assertEquals("""claude --p "two words"""", commandLine("claude", listOf("--p", "two words")))
    }

    // A flag that removes a confirmation step is the one thing that must never be remembered
    // quietly, so it is named rather than merely stored.
    @Test
    fun flagsThatRemoveAConfirmationAreRecognised() {
        for (flag in listOf(
            "--dangerously-skip-permissions",
            "--dangerously-bypass-approvals-and-sandbox",
            "--yolo",
            "--full-auto",
            "--auto-approve",
            "--no-sandbox",
            "--DANGEROUSLY-skip-permissions",
        )) {
            assertTrue(bypassesSafety(flag), flag)
        }
        for (flag in listOf("--model", "opus", "--append-system-prompt", "-p", "--sandbox")) {
            assertFalse(bypassesSafety(flag), flag)
        }
    }
}
