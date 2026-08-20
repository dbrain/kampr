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

fun typography(fonts: KamprFonts, label: LabelSpec, scale: TypeScale): KamprType {
    val ui = fonts.ui
    val mono = fonts.mono
    val phone = scale == TypeScale.Phone
    fun s(p: Double, d: Double) = (if (phone) p else d).sp
    return KamprType(
        screenTitle = TextStyle(fontFamily = ui, fontSize = s(26.0, 19.0), fontWeight = FontWeight.W800, letterSpacing = (-0.02).em),
        paneTitle = TextStyle(fontFamily = ui, fontSize = s(16.0, 15.5), fontWeight = FontWeight.W700),
        cardTitle = TextStyle(fontFamily = ui, fontSize = s(15.0, 12.5), fontWeight = FontWeight.W600),
        cardTitleQuiet = TextStyle(fontFamily = ui, fontSize = s(15.0, 12.5), fontWeight = FontWeight.W500),
        sectionLabel = TextStyle(
            fontFamily = ui,
            fontSize = s(13.0, 11.5),
            fontWeight = label.weight,
            letterSpacing = label.tracking,
        ),
        body = TextStyle(fontFamily = ui, fontSize = s(13.5, 13.0), fontWeight = FontWeight.W400, lineHeight = s(20.5, 19.5)),
        bodyStrong = TextStyle(fontFamily = ui, fontSize = s(14.0, 13.5), fontWeight = FontWeight.W700),
        caption = TextStyle(fontFamily = ui, fontSize = s(12.0, 11.5), fontWeight = FontWeight.W500, lineHeight = s(17.0, 16.5)),
        captionSmall = TextStyle(fontFamily = ui, fontSize = s(11.5, 11.0), fontWeight = FontWeight.W500, lineHeight = s(16.5, 16.0)),
        micro = TextStyle(fontFamily = ui, fontSize = s(11.0, 10.0), fontWeight = FontWeight.W500),
        meta = TextStyle(fontFamily = mono, fontSize = s(10.5, 9.5), fontWeight = FontWeight.W400),
        metaSmall = TextStyle(fontFamily = mono, fontSize = s(10.0, 9.5), fontWeight = FontWeight.W500),
        badge = TextStyle(fontFamily = ui, fontSize = s(11.5, 11.0), fontWeight = FontWeight.W700),
        button = TextStyle(fontFamily = ui, fontSize = s(15.0, 13.0), fontWeight = FontWeight.W700),
        buttonSmall = TextStyle(fontFamily = ui, fontSize = s(13.0, 12.0), fontWeight = FontWeight.W700),
        tab = TextStyle(fontFamily = ui, fontSize = s(12.5, 12.0), fontWeight = FontWeight.W700),
        key = TextStyle(fontFamily = mono, fontSize = s(10.0, 10.0), fontWeight = FontWeight.W500),
        pill = TextStyle(fontFamily = ui, fontSize = s(12.0, 11.0), fontWeight = FontWeight.W600),
    )
}
