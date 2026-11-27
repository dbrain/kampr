package dev.kampr.shared

import dev.kampr.shared.util.bypassesSafety
import dev.kampr.shared.util.commandLine
import dev.kampr.shared.util.displayArgv
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

    // docs/25: a stored command is shown before it is run, so what is shown has to be exactly what
    // runs. An element is one argument whatever is in it, and nothing in one may move the cursor,
    // reorder the line or hide a character.
    @Test
    fun aStoredCommandIsShownEscapedOneArgumentPerWord() {
        val cases = listOf(
            listOf("nvim", "-S", "/home/u/s.vim") to "nvim -S /home/u/s.vim",
            listOf("echo", "two words") to "echo 'two words'",
            listOf("sh", "-c", "a; b") to "sh -c 'a; b'",
            listOf("echo", "$(reboot)") to "echo '$(reboot)'",
            listOf("echo", "it's") to "echo 'it'\\''s'",
            listOf("echo", "") to "echo ''",
            listOf("echo", "a\nb") to "echo $'a\\nb'",
            listOf("printf", "\u001b[2J") to "printf $'\\x1b[2J'",
            listOf("cat", "evil\u202Etxt.sh") to "cat $'evil\\u{202e}txt.sh'",
            listOf("echo", "café") to "echo 'café'",
        )
        for ((argv, shown) in cases) assertEquals(shown, displayArgv(argv), "argv: $argv")
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
