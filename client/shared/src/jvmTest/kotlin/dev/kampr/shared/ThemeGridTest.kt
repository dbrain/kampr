package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpRect
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.AppearanceScreen
import dev.kampr.shared.ui.COLUMN_MAX
import dev.kampr.shared.ui.THEME_COLUMN_MIN
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// Every theme card announces itself as "<title> theme — <credit>", which is the one string that is
// on exactly one node per card.
private const val CARD = " theme — "

// The sentence that says what the whole screen is for.
private const val BLURB = "Soft native ships. The rest stay one attribute away."

@OptIn(ExperimentalTestApi::class)
private fun body(width: Dp, height: Dp = 1400.dp, block: SkikoComposeUiTest.() -> Unit) =
    runSkikoComposeUiTest(Size(width.value, height.value), Density(1f)) {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.fillMaxSize()) {
                    AppearanceScreen(themeOf("soft").id, ThemeMode.Dark, {}, {}, {})
                }
            }
        }
        waitForIdle()
        block()
    }

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteractionsProvider.cards(): List<DpRect> {
    val found = onAllNodesWithContentDescription(CARD, substring = true)
    return found.fetchSemanticsNodes().indices.map { found[it].getUnclippedBoundsInRoot() }
}

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteractionsProvider.ellipsised(text: String): Boolean =
    onAllNodesWithText(text, substring = true, useUnmergedTree = true).fetchSemanticsNodes().any { node ->
        val laid = mutableListOf<TextLayoutResult>()
        if (!node.config.contains(SemanticsActions.GetTextLayoutResult)) return@any false
        node.config[SemanticsActions.GetTextLayoutResult].action?.invoke(laid)
        laid.any { it.multiParagraph.didExceedMaxLines }
    }

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteractionsProvider.creditsOnOneLine(): Boolean =
    AllThemes.all { spec ->
        onAllNodesWithText(spec.id.credit, substring = true, useUnmergedTree = true)
            .fetchSemanticsNodes().all { node ->
                val laid = mutableListOf<TextLayoutResult>()
                if (!node.config.contains(SemanticsActions.GetTextLayoutResult)) return@all true
                node.config[SemanticsActions.GetTextLayoutResult].action?.invoke(laid)
                laid.all { it.lineCount <= 1 }
            }
    }

// The grid hardcoded its column count per breakpoint — four on any desktop at all — so a 900 dp
// window, whose body is about 600 dp once the sidebar has had its share, laid four cards out at a
// width no theme card's own content survives.
//
// `THEME_COLUMN_MIN` is measured the way `COLUMN_MAX` was, from the other end: a ladder of card
// widths rendered with the real font files, watching for the first one at which every theme's
// credit line still sits on one line.
@OptIn(ExperimentalTestApi::class)
class ThemeGridTest {
    @Test
    fun noWindowSqueezesAThemeCardBelowTheWidthItsOwnCreditNeeds() {
        for (width in listOf(411.dp, 600.dp, 720.dp, 900.dp, 1000.dp, 1200.dp, 1440.dp, 1920.dp, 3440.dp)) {
            body(width) {
                val narrowest = cards().minOf { it.right - it.left }
                assertTrue(
                    narrowest >= THEME_COLUMN_MIN,
                    "a $width body laid its theme cards out at $narrowest, under the $THEME_COLUMN_MIN their content needs",
                )
                assertTrue(creditsOnOneLine(), "a $width body wrapped a credit line onto a second row")
            }
        }
    }

    @Test
    fun theNumberOfThemeColumnsAnswersTheWindowRatherThanTheBreakpointItIsIn() {
        val counted = linkedMapOf<Dp, Int>()
        for (width in listOf(411.dp, 720.dp, 1100.dp, 1440.dp, 3440.dp)) {
            body(width) { counted[width] = cards().map { it.left }.distinct().size }
        }
        assertEquals(1, counted[411.dp], "a phone with room for one card was given more than one")
        assertTrue(
            counted.values.zipWithNext().all { (narrow, wide) -> wide >= narrow },
            "the count went backwards as the window grew: $counted",
        )
        assertTrue(
            counted[3440.dp]!! > counted[720.dp]!!,
            "3440 dp laid out the same number of columns as 720 dp, so the count ignores the window: $counted",
        )
        // The room has to be used, not just respected. A 1440 dp body is 1400 dp of grid, which is
        // four columns of 339 dp — comfortably over the 295 dp measure — so laying out fewer is a
        // screen answering the maximum measure instead of the window it was given.
        assertEquals(
            4,
            counted[1440.dp],
            "1440 dp has room for four cards of 339 dp, well over $THEME_COLUMN_MIN, " +
                "and laid out ${counted[1440.dp]}",
        )
    }

    // The header is a back arrow, the screen's name and a sentence about the grid under it, all on
    // one row with the sentence taking whatever the other two leave. On a 600 dp body that is 87 dp
    // and the sentence ends "...The rest stay one a…", which is the screen's own explanation cut
    // off mid-word on every window narrower than about a thousand dp.
    @Test
    fun theSentenceThatSaysWhatTheScreenIsForIsNeverCutOff() {
        for (width in listOf(411.dp, 600.dp, 720.dp, 1000.dp, 1440.dp)) {
            body(width) {
                assertTrue(!ellipsised(BLURB), "a $width body ellipsised the line that explains the screen")
            }
        }
    }

    // The same rule the setup screen was already given: a caption belongs to the column it
    // describes, and one left at the window's edge while its grid sits 660 dp away has been left
    // behind. `aWideDesktopPutsTheTwoSettingsMeasuresBesideEachOtherNotAtOppositeEdges` names it.
    @Test
    fun theHeaderTravelsWithTheGridItDescribesRatherThanStayingAtTheWindowsEdge() = body(3440.dp) {
        val grid = cards().minOf { it.left }
        val header = onNodeWithContentDescription("Back").getUnclippedBoundsInRoot().left
        assertTrue(
            abs((header - grid).value) <= 24f,
            "the header starts at $header and the grid it heads at $grid, so it was left behind at the window's edge",
        )
    }

    // The other end is already measured and must not move: past its own measure a theme card is
    // stretch, so the widest window still stops at COLUMN_MAX rather than filling itself.
    // Four themes in three columns is three and then one, with a hole where the fourth would be.
    @Test
    fun aWindowThatFitsThreeColumnsLaysTheFourThemesOutAsTwoRowsOfTwo() = body(1100.dp) {
        assertEquals(
            2,
            cards().map { it.left }.distinct().size,
            "1100 dp fits three columns, and four themes in three columns is a card on its own",
        )
    }

    @Test
    fun anUltrawideStopsAtTheMeasureTheCardEndsAtAndSitsInTheMiddle() = body(3440.dp) {
        val cards = cards()
        val widest = cards.maxOf { it.right - it.left }
        assertTrue(widest <= COLUMN_MAX, "a 3440 dp body stretched a theme card to $widest, past $COLUMN_MAX")
        val lefts = cards.map { it.left }.distinct().sorted()
        val leading = lefts.first()
        val trailing = 3440.dp - cards.maxOf { it.right }
        assertTrue(
            abs((leading - trailing).value) <= 24f,
            "the grid sits $leading from the left and $trailing from the right, so it is not centred",
        )
    }
}
