package dev.kampr.terminal.spike

sealed interface ColorSpec {
    data object Default : ColorSpec
    data class Indexed(val v: Int) : ColorSpec
    data class Rgb(val r: Int, val g: Int, val b: Int) : ColorSpec
}

data class Style(
    val fg: ColorSpec = ColorSpec.Default,
    val bg: ColorSpec = ColorSpec.Default,
    val bold: Boolean = false,
    val dim: Boolean = false,
    val italic: Boolean = false,
    val underline: Boolean = false,
    val blink: Boolean = false,
    val reverse: Boolean = false,
    val strike: Boolean = false,
    val hidden: Boolean = false,
)

class Run(val s: Int, val x: String, val l: Int? = null)

class RowDiff(val row: Int, val runs: List<Run>)

class CursorPos(val col: Int, val row: Int, val visible: Boolean)

sealed interface ServerMsg

class StylesMsg(val from: Int, val styles: List<Style>) : ServerMsg

class GridReset(
    val cols: Int,
    val rows: Int,
    val rowsData: List<RowDiff>,
    val cursor: CursorPos,
) : ServerMsg

class GridPatch(val rows: List<RowDiff>, val cursor: CursorPos) : ServerMsg
