@file:OptIn(InternalResourceApi::class)

package dev.kampr.terminal.spike.res

import kotlin.OptIn
import kotlin.String
import kotlin.collections.MutableMap
import org.jetbrains.compose.resources.FontResource
import org.jetbrains.compose.resources.InternalResourceApi
import org.jetbrains.compose.resources.ResourceContentHash
import org.jetbrains.compose.resources.ResourceItem

private const val MD: String = "composeResources/dev.kampr.terminal.spike.res/"

@delegate:ResourceContentHash(444_542_743)
public val Res.font.jetbrainsmononl_bold: FontResource by lazy {
      FontResource("font:jetbrainsmononl_bold", setOf(
        ResourceItem(setOf(), "${MD}font/jetbrainsmononl_bold.ttf", -1, -1),
      ))
    }

@delegate:ResourceContentHash(296_050_529)
public val Res.font.jetbrainsmononl_bolditalic: FontResource by lazy {
      FontResource("font:jetbrainsmononl_bolditalic", setOf(
        ResourceItem(setOf(), "${MD}font/jetbrainsmononl_bolditalic.ttf", -1, -1),
      ))
    }

@delegate:ResourceContentHash(1_999_224_707)
public val Res.font.jetbrainsmononl_italic: FontResource by lazy {
      FontResource("font:jetbrainsmononl_italic", setOf(
        ResourceItem(setOf(), "${MD}font/jetbrainsmononl_italic.ttf", -1, -1),
      ))
    }

@delegate:ResourceContentHash(-1_105_414_507)
public val Res.font.jetbrainsmononl_regular: FontResource by lazy {
      FontResource("font:jetbrainsmononl_regular", setOf(
        ResourceItem(setOf(), "${MD}font/jetbrainsmononl_regular.ttf", -1, -1),
      ))
    }

@InternalResourceApi
internal fun _collectCommonMainFont0Resources(map: MutableMap<String, FontResource>) {
  map.put("jetbrainsmononl_bold", Res.font.jetbrainsmononl_bold)
  map.put("jetbrainsmononl_bolditalic", Res.font.jetbrainsmononl_bolditalic)
  map.put("jetbrainsmononl_italic", Res.font.jetbrainsmononl_italic)
  map.put("jetbrainsmononl_regular", Res.font.jetbrainsmononl_regular)
}
