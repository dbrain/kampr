package dev.kampr.terminal.bench

import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Style

enum class Profile { Idle, Mixed, Scroll, Worst }

class Rng(private var state: Long = 0x9E3779B97F4A7C15uL.toLong()) {
    fun next(): Int {
        state = state xor (state shl 13)
        state = state xor (state ushr 7)
        state = state xor (state shl 17)
        return (state ushr 32).toInt() and 0x7FFFFFFF
    }

    fun int(bound: Int) = next() % bound

    fun reseed(value: Long) {
        state = if (value == 0L) 1L else value
    }
}

object BenchStyles {
    const val DEFAULT = 0
    const val DIM = 1
    const val GREEN = 2
    const val YELLOW = 3
    const val RED = 4
    const val BLUE = 5
    const val CYAN = 6
    const val MAGENTA = 7
    const val BOLD = 8
    const val BOLD_GREEN = 9
    const val BOLD_BLUE = 10
    const val ITALIC_DIM = 11
    const val UNDERLINE_BLUE = 12
    const val REVERSE = 13
    const val DIFF_ADD = 14
    const val DIFF_DEL = 15
    const val TRUECOLOR_BASE = 16
    const val TRUECOLOR_COUNT = 24
    const val COUNT = TRUECOLOR_BASE + TRUECOLOR_COUNT

    fun table(): ServerMsg.Styles {
        val styles = ArrayList<Style>(COUNT)
        styles.add(Style())
        styles.add(Style(dim = true))
        styles.add(Style(fg = ColorSpec.Indexed(10)))
        styles.add(Style(fg = ColorSpec.Indexed(11)))
        styles.add(Style(fg = ColorSpec.Indexed(9)))
        styles.add(Style(fg = ColorSpec.Indexed(12)))
        styles.add(Style(fg = ColorSpec.Indexed(14)))
        styles.add(Style(fg = ColorSpec.Indexed(13)))
        styles.add(Style(bold = true))
        styles.add(Style(fg = ColorSpec.Indexed(10), bold = true))
        styles.add(Style(fg = ColorSpec.Indexed(12), bold = true))
        styles.add(Style(italic = true, dim = true))
        styles.add(Style(fg = ColorSpec.Indexed(12), underline = true))
        styles.add(Style(reverse = true))
        styles.add(Style(fg = ColorSpec.Rgb(126, 231, 135), bg = ColorSpec.Rgb(16, 38, 20)))
        styles.add(Style(fg = ColorSpec.Rgb(255, 123, 114), bg = ColorSpec.Rgb(44, 16, 18)))
        for (i in 0 until TRUECOLOR_COUNT) {
            val h = i * 15
            styles.add(
                Style(fg = ColorSpec.Rgb(120 + (h % 136), 90 + ((h * 7) % 166), 140 + ((h * 3) % 116))),
            )
        }
        return ServerMsg.Styles(0, styles)
    }
}

private val WORDS = arrayOf(
    "kampr", "herdr", "observe", "pane", "grid", "reset", "patch", "style", "buffer", "emulator",
    "socket", "reconnect", "backoff", "scrollback", "cursor", "viewport", "compose", "canvas",
    "wasm", "android", "render", "frame", "budget", "dropped", "shaping", "glyph", "atlas",
    "supervisor", "registry", "provider", "session", "layout", "geometry", "dirty", "runs",
)

private val PATHS = arrayOf(
    "crates/kampr-term/src/grid.rs", "crates/kampr-herdr/src/observe.rs",
    "client/terminal/render/GridRenderer.kt", "crates/kampr-node/src/ws.rs",
    "docs/04-wire-protocol.md", "crates/kampr-core/src/registry.rs",
)

private val LEVELS = arrayOf("TRACE", "DEBUG", "INFO ", "WARN ", "ERROR")
private val LEVEL_STYLES = intArrayOf(
    BenchStyles.DIM, BenchStyles.CYAN, BenchStyles.GREEN, BenchStyles.YELLOW, BenchStyles.RED,
)

private const val CHURN_ALPHABET =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ./:_-()[]{}<>=+*#@%&|~^\$?!,;'\"`\\"

class LineFactory(private val rng: Rng) {
    private val builder = StringBuilder(256)

    fun line(cols: Int, kind: Int, seq: Int): List<Run> = when (kind) {
        0 -> log(cols, seq)
        1 -> prompt(seq)
        2 -> diff(cols, seq)
        3 -> code(seq)
        4 -> progress(cols, seq)
        else -> boxed(seq)
    }

    fun churn(cols: Int): List<Run> {
        val runs = ArrayList<Run>(12)
        var col = 0
        while (col < cols) {
            val length = (3 + rng.int(9)).coerceAtMost(cols - col)
            builder.setLength(0)
            repeat(length) { builder.append(CHURN_ALPHABET[rng.int(CHURN_ALPHABET.length)]) }
            runs.add(Run(rng.int(BenchStyles.COUNT), builder.toString()))
            col += length
        }
        return runs
    }

    private fun words(n: Int): String {
        builder.setLength(0)
        repeat(n) {
            if (builder.isNotEmpty()) builder.append(' ')
            builder.append(WORDS[rng.int(WORDS.size)])
        }
        return builder.toString()
    }

    private fun log(cols: Int, seq: Int): List<Run> {
        val level = rng.int(LEVELS.size)
        val t = seq % 86400
        val stamp = two(t / 3600) + ":" + two(t / 60 % 60) + ":" + two(t % 60) + "." + three(seq % 1000)
        return listOf(
            Run(BenchStyles.DIM, "$stamp "),
            Run(LEVEL_STYLES[level], LEVELS[level] + " "),
            Run(BenchStyles.BLUE, PATHS[rng.int(PATHS.size)] + ": "),
            Run(BenchStyles.DEFAULT, words(3 + rng.int(5)).take(maxOf(0, cols - 60))),
        )
    }

    private fun prompt(seq: Int): List<Run> = listOf(
        Run(BenchStyles.BOLD_GREEN, "❯ "),
        Run(BenchStyles.DEFAULT, "cargo "),
        Run(BenchStyles.CYAN, "test "),
        Run(BenchStyles.DEFAULT, "-p "),
        Run(BenchStyles.MAGENTA, "kampr-term "),
        Run(BenchStyles.ITALIC_DIM, "# $seq " + words(2)),
    )

    private fun diff(cols: Int, seq: Int): List<Run> {
        val add = seq % 2 == 0
        val style = if (add) BenchStyles.DIFF_ADD else BenchStyles.DIFF_DEL
        val body = (if (add) "+ " else "- ") + words(4 + rng.int(4))
        return listOf(Run(style, body.padEnd(cols).take(cols)))
    }

    private fun code(seq: Int): List<Run> = listOf(
        Run(BenchStyles.DIM, four(seq % 9999) + " "),
        Run(BenchStyles.MAGENTA, "fn "),
        Run(BenchStyles.BOLD_BLUE, WORDS[rng.int(WORDS.size)]),
        Run(BenchStyles.DEFAULT, "(buf: &mut "),
        Run(BenchStyles.YELLOW, "CellBuffer"),
        Run(BenchStyles.DEFAULT, ", n: "),
        Run(BenchStyles.YELLOW, "usize"),
        Run(BenchStyles.DEFAULT, ") -> "),
        Run(BenchStyles.YELLOW, "Result"),
        Run(BenchStyles.DEFAULT, "<()> {"),
    )

    private fun progress(cols: Int, seq: Int): List<Run> {
        val width = (cols - 20).coerceAtLeast(4)
        val filled = (seq * 3) % (width + 1)
        builder.setLength(0)
        repeat(filled) { builder.append('█') }
        val done = builder.toString()
        builder.setLength(0)
        repeat(width - filled) { builder.append('░') }
        return listOf(
            Run(BenchStyles.DIM, "build "),
            Run(BenchStyles.TRUECOLOR_BASE + (seq % BenchStyles.TRUECOLOR_COUNT), done),
            Run(BenchStyles.DIM, builder.toString()),
            Run(BenchStyles.BOLD, " " + three(filled * 100 / width) + "%"),
        )
    }

    private fun boxed(seq: Int): List<Run> = listOf(
        Run(BenchStyles.DIM, "│ "),
        Run(BenchStyles.UNDERLINE_BLUE, "https://herdr.dev/docs/" + WORDS[rng.int(WORDS.size)]),
        Run(BenchStyles.DEFAULT, "  "),
        Run(if (seq % 3 == 0) BenchStyles.GREEN else BenchStyles.RED, if (seq % 3 == 0) "✓" else "✗"),
        Run(BenchStyles.DIM, " │"),
    )

    private fun two(v: Int) = if (v < 10) "0$v" else v.toString()
    private fun three(v: Int) = v.toString().padStart(3, '0')
    private fun four(v: Int) = v.toString().padStart(4, ' ')
}

class Workload(private var profile: Profile, private var cols: Int, private var rows: Int) {
    private val rng = Rng()
    private val factory = LineFactory(rng)
    private var seq = 0
    private var rowBudget = 0.0
    private var sinceReset = 0.0
    private var scrolled = ArrayList<List<Run>>()
    private var cursorCol = 0

    val lines: LineFactory get() = factory

    fun reconfigure(profile: Profile, cols: Int, rows: Int) {
        this.profile = profile
        this.cols = cols
        this.rows = rows
        rng.reseed(0x51ED270Bu.toLong())
        seq = 0
        rowBudget = 0.0
        sinceReset = 1e9
        scrolled = ArrayList()
    }

    fun step(paneId: String, dtMs: Double, nowMs: Double): List<ServerMsg> {
        val blink = ((nowMs / 530.0).toInt() % 2) == 0
        val out = ArrayList<ServerMsg>(2)
        when (profile) {
            Profile.Idle -> {
                cursorCol = (seq / 30) % cols
                seq++
                out.add(ServerMsg.GridPatch(paneId, emptyList(), Cursor(cursorCol, rows - 1, blink), emptyList()))
            }

            Profile.Mixed -> {
                sinceReset += dtMs
                if (sinceReset >= 2000.0) {
                    sinceReset = 0.0
                    out.add(reset(paneId, blink))
                } else {
                    rowBudget += 40.0 * dtMs / 1000.0
                    val n = rowBudget.toInt()
                    rowBudget -= n
                    val diffs = ArrayList<RowDiff>(n)
                    repeat(n) {
                        diffs.add(RowDiff(rng.int(rows), factory.line(cols, seq % 6, seq)))
                        seq++
                    }
                    cursorCol = (cursorCol + 1) % cols
                    out.add(ServerMsg.GridPatch(paneId, diffs, Cursor(cursorCol, rows - 1, blink), emptyList()))
                }
            }

            Profile.Scroll -> {
                if (scrolled.size != rows) {
                    scrolled = ArrayList(rows)
                    repeat(rows) { scrolled.add(factory.line(cols, seq++ % 6, seq)) }
                }
                rowBudget += 30.0 * dtMs / 1000.0
                val n = rowBudget.toInt()
                rowBudget -= n
                repeat(n) {
                    scrolled.removeAt(0)
                    scrolled.add(factory.line(cols, seq % 6, seq))
                    seq++
                }
                val diffs = if (n > 0) (0 until rows).map { RowDiff(it, scrolled[it]) } else emptyList()
                out.add(ServerMsg.GridPatch(paneId, diffs, Cursor(cursorCol, rows - 1, blink), emptyList()))
            }

            Profile.Worst -> {
                val diffs = (0 until rows).map { RowDiff(it, factory.churn(cols)) }
                seq++
                cursorCol = (cursorCol + 1) % cols
                out.add(ServerMsg.GridPatch(paneId, diffs, Cursor(cursorCol, rows - 1, blink), emptyList()))
            }
        }
        return out
    }

    fun reset(paneId: String, blink: Boolean): ServerMsg.GridReset {
        val diffs = ArrayList<RowDiff>(rows)
        for (row in 0 until rows) {
            diffs.add(RowDiff(row, factory.line(cols, seq % 6, seq)))
            seq++
        }
        return ServerMsg.GridReset(paneId, cols, rows, diffs, Cursor(cursorCol, rows - 1, blink), emptyList())
    }

    fun history(paneId: String, depth: Int): ServerMsg.Scrollback {
        val diffs = ArrayList<RowDiff>(depth)
        for (row in 0 until depth) diffs.add(RowDiff(row, factory.line(cols, row % 6, row)))
        return ServerMsg.Scrollback(paneId, 0, diffs, depth, complete = true, capped = false)
    }
}
