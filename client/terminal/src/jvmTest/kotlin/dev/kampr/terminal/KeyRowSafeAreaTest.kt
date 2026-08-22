package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.isSpecified
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.BottomEdgeHeldBelow
import dev.kampr.shared.ui.BottomNav
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.Tab
import dev.kampr.shared.ui.keyboardInset
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.input.keyRowPadding
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

// Rotated, with three-button navigation: the strip the system reserves leaves the bottom of the
// window and lands on one side of it, which is where the outermost caps are.
private val SIDE_BARS = listOf(
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 0.dp, right = 48.dp),
)

// Gboard on a 1080x2400 phone. `bottom` is zero because an open keyboard is drawn over the
// navigation bar and `KeyboardFloor` has already taken it — SafeAreaValueTest is what pins that.
private val KEYBOARD = SafeArea(top = 32.dp, bottom = 0.dp, ime = 300.dp)

private const val PANE = "01JNODE/w1:p1"

private object SilentIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun testTokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

private val LAST_ROW = listOf("End", "Left arrow", "Down arrow", "Right arrow")

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.lowestCap(): Dp = LAST_ROW
    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
    .also { assertTrue(it.isNotEmpty(), "no key caps on screen, so nothing was measured") }
    .maxOf { with(density) { it.boundsInRoot.bottom.toDp() } }

// The shape the phone actually stacks, whichever way round: the app root pays the keyboard once,
// the screen sits inside it, and the bottom navigation — when there is one — sits under the screen
// and takes the bottom edge off it. `nav` is the only thing that moves between the two containers
// this row lives in: the pane screen wears one, the mosaic switcher wears nothing at all.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.keyRow(
    compact: Boolean,
    bars: SafeArea = BARS,
    nav: Boolean = false,
    barTop: MutableState<Dp> = mutableStateOf(Dp.Unspecified),
): MutableState<Dp> {
    val pane = PaneState(PANE, StyleTable())
    val session = PaneSession(PANE)
    val navTop = mutableStateOf(Dp.Unspecified)
    setContent {
        CompositionLocalProvider(LocalTokens provides testTokens(), LocalSafeArea provides bars) {
            Box(Modifier.fillMaxSize().keyboardInset()) {
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f)) {
                        BottomEdgeHeldBelow(nav) {
                            Column(Modifier.fillMaxSize()) {
                                Box(Modifier.weight(1f))
                                val rowDensity = LocalDensity.current
                                PaneKeyRow(
                                    session,
                                    InputSink(pane.id, SilentIo, session.latches),
                                    compact,
                                    enabled = true,
                                    modifier = Modifier.onGloballyPositioned {
                                        barTop.value = with(rowDensity) { it.positionInRoot().y.toDp() }
                                    },
                                )
                            }
                        }
                    }
                    if (nav) {
                        val density = LocalDensity.current
                        Box(
                            Modifier.onGloballyPositioned {
                                navTop.value = with(density) { it.positionInRoot().y.toDp() }
                            },
                        ) {
                            BottomNav(Tab.Herd, {})
                        }
                    }
                }
            }
        }
    }
    waitForIdle()
    return navTop
}

// The key row docks above whatever the system is drawing at the bottom of the window — and above
// nothing at all when something else already covers it. Both halves matter: padding for a gesture
// handle that a navigation bar has already cleared leaves a dead strip under the last row of keys.
@OptIn(ExperimentalTestApi::class)
class KeyRowSafeAreaTest {
    @Test
    fun theKeyRowClearsTheGestureHandleWhenItIsTheLastThingInTheWindow() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                keyRow(compact)
                val screen = onRoot().getUnclippedBoundsInRoot()
                assertTrue(
                    lowestCap() <= screen.bottom - BARS.bottom,
                    "compact=$compact: the keys reach ${lowestCap()} of ${screen.bottom}, inside " +
                        "the ${BARS.bottom} the system draws its gesture handle in",
                )
            }
        }
    }

    @Test
    fun theKeyRowClearsTheBarsThatMoveToTheSideInLandscape() {
        for (bars in SIDE_BARS) {
            runComposeUiTest {
                keyRow(compact = true, bars = bars)
                val screen = onRoot().getUnclippedBoundsInRoot()
                val caps = LAST_ROW
                    .flatMap { onAllNodesWithContentDescription(it, substring = true).fetchSemanticsNodes() }
                    .also { assertTrue(it.isNotEmpty(), "no key caps on screen, so nothing was measured") }
                    .map { with(density) { it.boundsInRoot.left.toDp() to it.boundsInRoot.right.toDp() } }
                assertTrue(
                    caps.minOf { it.first } >= bars.left,
                    "$bars: a cap starts at ${caps.minOf { it.first }}, inside the ${bars.left} " +
                        "the system draws in on the left",
                )
                assertTrue(
                    caps.maxOf { it.second } <= screen.right - bars.right,
                    "$bars: a cap reaches ${caps.maxOf { it.second }} of ${screen.right}, inside " +
                        "the ${bars.right} the system draws in on the right",
                )
            }
        }
    }

    // The other half, and the one a fix for the first half breaks: on the pane screen the bottom
    // navigation is under the key row and is already holding the handle off, so a row that pays
    // again leaves a dead strip between the last key and the tabs. What it still owes there is its
    // own padding and nothing else — the same as it owes the keyboard.
    @Test
    fun theKeyRowAddsNothingButItsOwnPaddingOverTheBottomNavigation() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                val navTop = keyRow(compact, nav = true)
                assertTrue(navTop.value.isSpecified, "the bottom navigation never laid out")
                val gap = navTop.value - lowestCap()
                assertTrue(
                    abs((gap - keyRowPadding(compact)).value) < 1f,
                    "compact=$compact: $gap of strip between the last key and the bottom " +
                        "navigation at ${navTop.value}, which is already holding the handle off — " +
                        "the row owes it ${keyRowPadding(compact)} and no more",
                )
            }
        }
    }

    // The report, verbatim: "keyboard opens up on terminal, there's a space between keyboard and
    // the ctrl/pgdown/pgend keys". The keys are what the row docks on, and the tabs stand down
    // while it is up, so the row is the last thing in the window again.
    //
    // And then, verbatim again: "now there's no padding at all". Docking on the keys is not the
    // same as touching them. `safe.bottom` was doing two jobs — clear the system chrome, *and* be
    // the bar's own bottom padding — so the moment the keyboard took the first job away the second
    // went with it and the last row of caps sat flush on Gboard's first.
    @Test
    fun theKeyRowRestsOnTheKeysWithItsOwnPaddingAndNoMore() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                keyRow(compact, bars = KEYBOARD)
                val screen = onRoot().getUnclippedBoundsInRoot()
                val gap = screen.bottom - KEYBOARD.ime - lowestCap()
                assertTrue(
                    abs((gap - keyRowPadding(compact)).value) < 1f,
                    "compact=$compact: $gap between the last key and the keys, which start at " +
                        "${screen.bottom - KEYBOARD.ime} of ${screen.bottom} — the row owes them " +
                        "${keyRowPadding(compact)}, the same as it leaves above its first row",
                )
            }
        }
    }

    // The bar's own padding is symmetric, and that is the whole of the rule: what is above the
    // first row of caps is what is below the last. Anything else is a number picked to look right
    // against one piece of chrome, which is how the two corrections that came before this one both
    // went wrong.
    @Test
    fun theKeyRowLeavesTheSameRoomBelowItsCapsAsAbove() {
        for (compact in listOf(false, true)) {
            runComposeUiTest {
                val barTop = mutableStateOf(Dp.Unspecified)
                keyRow(compact, bars = BARS, barTop = barTop)
                assertTrue(barTop.value.isSpecified, "the key row never laid out")
                val screen = onRoot().getUnclippedBoundsInRoot()
                val firstCap = onNodeWithContentDescription("Escape key", substring = true)
                    .getUnclippedBoundsInRoot()
                val above = firstCap.top - barTop.value
                val below = screen.bottom - BARS.bottom - lowestCap()
                assertTrue(
                    abs((above - below).value) < 1f,
                    "compact=$compact: $above above the caps and $below below them",
                )
            }
        }
    }
}
