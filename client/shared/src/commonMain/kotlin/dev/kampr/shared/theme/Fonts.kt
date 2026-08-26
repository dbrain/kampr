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
import dev.kampr.shared.res.manrope_400
import dev.kampr.shared.res.manrope_500
import dev.kampr.shared.res.manrope_600
import dev.kampr.shared.res.manrope_700
import dev.kampr.shared.res.manrope_800
import dev.kampr.shared.res.terminalmono_bold
import dev.kampr.shared.res.terminalmono_bolditalic
import dev.kampr.shared.res.terminalmono_italic
import dev.kampr.shared.res.terminalmono_regular

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
// JetBrains Mono NL, with the symbols it does not carry added from the Noto families (all
// OFL 1.1): a browser has no system font behind Skia, and a FontFamily of loaded fonts resolves
// to exactly one typeface, so a codepoint the face lacks — U+23F5 and U+273B are both in Claude's
// own status line — is tofu and nothing else can supply it. Added glyphs take the 600/1000 advance
// and carry JetBrains Mono's vertical metrics, so no symbol widens a cell or grows a line.
//
// Probe #271: the four faces are built by `tools/terminalmono.py`, which is the whole of how they
// exist — run `--verify` to check the shipped files against a rebuild. A codepoint in the box
// lattice is aliased onto JetBrains Mono's own glyph rather than cut in, because the lattice
// deliberately overflows the cell so neighbours join and a cut-in is centred inside it; that is
// also the only path that keeps the weight of the face. What the face must cover is measured, and
// the list lives in `shared/src/jvmTest/resources/agent-glyphs.txt`.
private val terminalFaces = listOf(
    FontFace("kmono-regular", Res.font.terminalmono_regular, FontWeight.W400),
    FontFace("kmono-bold", Res.font.terminalmono_bold, FontWeight.W700),
    FontFace("kmono-italic", Res.font.terminalmono_italic, FontWeight.W400, FontStyle.Italic),
    FontFace("kmono-bolditalic", Res.font.terminalmono_bolditalic, FontWeight.W700, FontStyle.Italic),
)

@Composable
fun resolveFonts(spec: ThemeSpec): KamprFonts? {
    val ui = rememberFamily(remember(spec.ui) { faces(spec.ui) })
    val mono = rememberFamily(remember(spec.mono) { faces(spec.mono) })
    val terminal = rememberFamily(terminalFaces)
    if (ui == null || mono == null || terminal == null) return null
    return KamprFonts(ui, mono, terminal)
}
