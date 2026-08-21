package dev.kampr.shared

import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.TextUnitType
import androidx.compose.ui.unit.isSpecified
import dev.kampr.shared.theme.AllFamilies
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprType
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)

private fun KamprType.named(): List<Pair<String, TextStyle>> = listOf(
    "screenTitle" to screenTitle,
    "paneTitle" to paneTitle,
    "cardTitle" to cardTitle,
    "cardTitleQuiet" to cardTitleQuiet,
    "sectionLabel" to sectionLabel,
    "body" to body,
    "bodyStrong" to bodyStrong,
    "caption" to caption,
    "captionSmall" to captionSmall,
    "micro" to micro,
    "meta" to meta,
    "metaSmall" to metaSmall,
    "badge" to badge,
    "button" to button,
    "buttonSmall" to buttonSmall,
    "tab" to tab,
    "key" to key,
    "pill" to pill,
)

private fun scales(): List<Pair<TypeScale, List<Pair<String, TextStyle>>>> =
    TypeScale.entries.map { scale ->
        scale to typography(fonts, AllFamilies.first().dark.label, scale).named()
    }

class TypeScaleTest {
    // The whole scale in `sp`. A `dp` size looks identical on the test device and then ignores the
    // reader who has turned their system font up, which is the failure no screenshot shows.
    @Test
    fun everySizeFollowsTheSystemFontScale() {
        for ((scale, tokens) in scales()) {
            for ((name, style) in tokens) {
                assertEquals(TextUnitType.Sp, style.fontSize.type, "$scale.$name font size")
                if (style.lineHeight.isSpecified) {
                    assertEquals(TextUnitType.Sp, style.lineHeight.type, "$scale.$name line height")
                }
            }
        }
    }

    // Android's floor, and the reason the app read as tiny: eleven of these were under it.
    @Test
    fun nothingIsSmallerThanTwelveSp() {
        for ((scale, tokens) in scales()) {
            for ((name, style) in tokens) {
                assertTrue(style.fontSize.value >= 12f, "$scale.$name is ${style.fontSize.value}sp")
            }
        }
    }

    // Material's body size. `body` is the reply box and every paragraph in the app; `button` and
    // `bodyStrong` are read at a glance and at arm's length.
    @Test
    fun readingSizesMeetMaterialBody() {
        val phone = typography(fonts, AllFamilies.first().dark.label, TypeScale.Phone)
        val desk = typography(fonts, AllFamilies.first().dark.label, TypeScale.Desk)
        for ((name, style) in listOf("body" to phone.body, "bodyStrong" to phone.bodyStrong, "button" to phone.button)) {
            assertTrue(style.fontSize.value >= 16f, "phone.$name is ${style.fontSize.value}sp")
        }
        for ((name, style) in listOf("body" to desk.body, "bodyStrong" to desk.bodyStrong, "button" to desk.button)) {
            assertTrue(style.fontSize.value >= 14f, "desk.$name is ${style.fontSize.value}sp")
        }
    }

    // A line box that does not grow with its text clips descenders the moment the reader scales up.
    @Test
    fun lineHeightsLeaveRoomForTheText() {
        for ((scale, tokens) in scales()) {
            for ((name, style) in tokens) {
                if (!style.lineHeight.isSpecified) continue
                assertTrue(
                    style.lineHeight.value >= style.fontSize.value * 1.25f,
                    "$scale.$name line height ${style.lineHeight.value} against ${style.fontSize.value}",
                )
            }
        }
    }

    // The sizes are a property of the scale, not of the skin: a theme that quietly shrank its own
    // text would put half the app back under the floor with nothing to catch it.
    @Test
    fun everyThemeGetsTheSameSizes() {
        for (scale in TypeScale.entries) {
            val first = typography(fonts, AllFamilies.first().dark.label, scale).named().map { it.second.fontSize }
            for (family in AllFamilies) {
                for (spec in listOf(family.dark, family.light)) {
                    val sizes = typography(fonts, spec.label, scale).named().map { it.second.fontSize }
                    assertEquals(first, sizes, "${family.id} $scale")
                }
            }
        }
    }
}
