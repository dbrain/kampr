package dev.kampr.shared.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.remember
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import dev.kampr.shared.res.Res
import dev.kampr.shared.res.archivo_500
import dev.kampr.shared.res.archivo_700
import dev.kampr.shared.res.archivo_900
import dev.kampr.shared.res.ibmplexmono_400
import dev.kampr.shared.res.ibmplexmono_500
import dev.kampr.shared.res.ibmplexmono_600
import dev.kampr.shared.res.instrumentsans_400
import dev.kampr.shared.res.instrumentsans_500
import dev.kampr.shared.res.instrumentsans_600
import dev.kampr.shared.res.jetbrainsmono_400
import dev.kampr.shared.res.jetbrainsmono_500
import dev.kampr.shared.res.jetbrainsmono_700
import dev.kampr.shared.res.jetbrainsmononl_bold
import dev.kampr.shared.res.jetbrainsmononl_bolditalic
import dev.kampr.shared.res.jetbrainsmononl_italic
import dev.kampr.shared.res.jetbrainsmononl_regular
import dev.kampr.shared.res.manrope_400
import dev.kampr.shared.res.manrope_500
import dev.kampr.shared.res.manrope_600
import dev.kampr.shared.res.manrope_700
import dev.kampr.shared.res.manrope_800

@Immutable
data class KamprFonts(val ui: FontFamily, val mono: FontFamily, val terminal: FontFamily)

private fun faces(id: FamilyId): List<FontFace> = when (id) {
    FamilyId.Manrope -> listOf(
        FontFace("manrope-400", Res.font.manrope_400, FontWeight.W400),
        FontFace("manrope-500", Res.font.manrope_500, FontWeight.W500),
        FontFace("manrope-600", Res.font.manrope_600, FontWeight.W600),
        FontFace("manrope-700", Res.font.manrope_700, FontWeight.W700),
        FontFace("manrope-800", Res.font.manrope_800, FontWeight.W800),
    )
    FamilyId.IbmPlexMono -> listOf(
        FontFace("plex-400", Res.font.ibmplexmono_400, FontWeight.W400),
        FontFace("plex-500", Res.font.ibmplexmono_500, FontWeight.W500),
        FontFace("plex-600", Res.font.ibmplexmono_600, FontWeight.W600),
    )
    FamilyId.JetBrainsMono -> listOf(
        FontFace("jbm-400", Res.font.jetbrainsmono_400, FontWeight.W400),
        FontFace("jbm-500", Res.font.jetbrainsmono_500, FontWeight.W500),
        FontFace("jbm-700", Res.font.jetbrainsmono_700, FontWeight.W700),
    )
    FamilyId.InstrumentSans -> listOf(
        FontFace("instrument-400", Res.font.instrumentsans_400, FontWeight.W400),
        FontFace("instrument-500", Res.font.instrumentsans_500, FontWeight.W500),
        FontFace("instrument-600", Res.font.instrumentsans_600, FontWeight.W600),
    )
    FamilyId.Archivo -> listOf(
        FontFace("archivo-500", Res.font.archivo_500, FontWeight.W500),
        FontFace("archivo-700", Res.font.archivo_700, FontWeight.W700),
        FontFace("archivo-900", Res.font.archivo_900, FontWeight.W900),
    )
}

// Probe #66: the ligature cut collapses two cells into one glyph inside a shaped run.
private val terminalFaces = listOf(
    FontFace("jbmnl-regular", Res.font.jetbrainsmononl_regular, FontWeight.W400),
    FontFace("jbmnl-bold", Res.font.jetbrainsmononl_bold, FontWeight.W700),
    FontFace("jbmnl-italic", Res.font.jetbrainsmononl_italic, FontWeight.W400, FontStyle.Italic),
    FontFace("jbmnl-bolditalic", Res.font.jetbrainsmononl_bolditalic, FontWeight.W700, FontStyle.Italic),
)

@Composable
fun resolveFonts(spec: ThemeSpec): KamprFonts? {
    val ui = rememberFamily(remember(spec.ui) { faces(spec.ui) })
    val mono = rememberFamily(remember(spec.mono) { faces(spec.mono) })
    val terminal = rememberFamily(terminalFaces)
    if (ui == null || mono == null || terminal == null) return null
    return KamprFonts(ui, mono, terminal)
}
