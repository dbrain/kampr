package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

@Immutable
data class KamprType(
    val screenTitle: TextStyle,
    val paneTitle: TextStyle,
    val cardTitle: TextStyle,
    val cardTitleQuiet: TextStyle,
    val sectionLabel: TextStyle,
    val body: TextStyle,
    val bodyStrong: TextStyle,
    val caption: TextStyle,
    val captionSmall: TextStyle,
    val micro: TextStyle,
    val meta: TextStyle,
    val metaSmall: TextStyle,
    val badge: TextStyle,
    val button: TextStyle,
    val buttonSmall: TextStyle,
    val tab: TextStyle,
    val key: TextStyle,
    val pill: TextStyle,
)

enum class TypeScale { Phone, Desk }

// The floor Android asks for. Material's own scale bottoms out at `labelSmall` 11sp and its
// accessibility guidance treats 12sp as the smallest text a phone should render; the Accessibility
// Scanner flags anything under it. This scale was drawn on a design canvas in CSS pixels, where
// 10–13 is ordinary, and it was carried across a unit at a time — which is how nine of these
// eighteen shipped under the floor, `body` shipped under Material's own body size, and the four
// most-used tokens in the app were all in the 10–12 range. On a real phone it read as tiny.
//
// Every size is `sp`, never `dp`: that is what makes the app follow the reader's system font size.
private const val FLOOR = 12.0

fun typography(fonts: KamprFonts, label: LabelSpec, scale: TypeScale): KamprType {
    val ui = fonts.ui
    val mono = fonts.mono
    val phone = scale == TypeScale.Phone
    fun s(p: Double, d: Double) = (if (phone) p else d).sp
    return KamprType(
        screenTitle = TextStyle(fontFamily = ui, fontSize = s(28.0, 21.0), fontWeight = FontWeight.W800, letterSpacing = (-0.02).em),
        paneTitle = TextStyle(fontFamily = ui, fontSize = s(18.0, 16.0), fontWeight = FontWeight.W700),
        cardTitle = TextStyle(fontFamily = ui, fontSize = s(16.0, 14.0), fontWeight = FontWeight.W600),
        cardTitleQuiet = TextStyle(fontFamily = ui, fontSize = s(16.0, 14.0), fontWeight = FontWeight.W500),
        sectionLabel = TextStyle(
            fontFamily = ui,
            fontSize = s(14.0, FLOOR),
            fontWeight = label.weight,
            letterSpacing = label.tracking,
        ),
        body = TextStyle(fontFamily = ui, fontSize = s(16.0, 14.0), fontWeight = FontWeight.W400, lineHeight = s(24.0, 21.0)),
        bodyStrong = TextStyle(fontFamily = ui, fontSize = s(16.0, 14.0), fontWeight = FontWeight.W700),
        caption = TextStyle(fontFamily = ui, fontSize = s(14.0, 13.0), fontWeight = FontWeight.W500, lineHeight = s(20.0, 18.5)),
        captionSmall = TextStyle(fontFamily = ui, fontSize = s(13.0, FLOOR), fontWeight = FontWeight.W500, lineHeight = s(18.0, 17.0)),
        micro = TextStyle(fontFamily = ui, fontSize = s(FLOOR, FLOOR), fontWeight = FontWeight.W500),
        meta = TextStyle(fontFamily = mono, fontSize = s(FLOOR, FLOOR), fontWeight = FontWeight.W400),
        metaSmall = TextStyle(fontFamily = mono, fontSize = s(FLOOR, FLOOR), fontWeight = FontWeight.W500),
        badge = TextStyle(fontFamily = ui, fontSize = s(FLOOR, FLOOR), fontWeight = FontWeight.W700),
        button = TextStyle(fontFamily = ui, fontSize = s(16.0, 14.0), fontWeight = FontWeight.W700),
        buttonSmall = TextStyle(fontFamily = ui, fontSize = s(14.0, 13.0), fontWeight = FontWeight.W700),
        tab = TextStyle(fontFamily = ui, fontSize = s(14.0, 13.0), fontWeight = FontWeight.W700),
        key = TextStyle(fontFamily = mono, fontSize = s(FLOOR, FLOOR), fontWeight = FontWeight.W500),
        pill = TextStyle(fontFamily = ui, fontSize = s(13.0, FLOOR), fontWeight = FontWeight.W600),
    )
}
