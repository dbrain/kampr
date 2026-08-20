package dev.kampr.terminal.spike

enum class Profile(val label: String) {
    MIXED("mixed 40 rows/s + reset/2s"),
    SCROLL("scroll 30 lines/s"),
    WORST("every row every frame"),
    IDLE("cursor blink only"),
}

class Rng(private var s: Long = 0x9E3779B97F4A7C15uL.toLong()) {
    fun next(): Int {
        s = s xor (s shl 13)
        s = s xor (s ushr 7)
        s = s xor (s shl 17)
        return (s ushr 32).toInt() and 0x7FFFFFFF
    }

    fun int(bound: Int) = next() % bound
    fun reseed(v: Long) {
        s = if (v == 0L) 1L else v
    }
}

object SpikeStyles {
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

    fun table(): StylesMsg {
        val s = ArrayList<Style>(COUNT)
        s.add(Style())
        s.add(Style(dim = true))
        s.add(Style(fg = ColorSpec.Indexed(10)))
        s.add(Style(fg = ColorSpec.Indexed(11)))
        s.add(Style(fg = ColorSpec.Indexed(9)))
        s.add(Style(fg = ColorSpec.Indexed(12)))
        s.add(Style(fg = ColorSpec.Indexed(14)))
        s.add(Style(fg = ColorSpec.Indexed(13)))
        s.add(Style(bold = true))
        s.add(Style(fg = ColorSpec.Indexed(10), bold = true))
        s.add(Style(fg = ColorSpec.Indexed(12), bold = true))
        s.add(Style(italic = true, dim = true))
        s.add(Style(fg = ColorSpec.Indexed(12), underline = true))
        s.add(Style(reverse = true))
        s.add(Style(fg = ColorSpec.Rgb(126, 231, 135), bg = ColorSpec.Rgb(16, 38, 20)))
        s.add(Style(fg = ColorSpec.Rgb(255, 123, 114), bg = ColorSpec.Rgb(44, 16, 18)))
        for (i in 0 until TRUECOLOR_COUNT) {
            val h = i * 15
            s.add(Style(fg = ColorSpec.Rgb(120 + (h % 136), 90 + ((h * 7) % 166), 140 + ((h * 3) % 116))))
        }
        return StylesMsg(0, s)
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
    "client/shared/terminal/Canvas.kt", "crates/kampr-node/src/ws.rs",
    "docs/04-wire-protocol.md", "crates/kampr-core/src/registry.rs",
)

private val LEVELS = arrayOf("TRACE", "DEBUG", "INFO ", "WARN ", "ERROR")
private val LEVEL_STYLES = intArrayOf(
    SpikeStyles.DIM, SpikeStyles.CYAN, SpikeStyles.GREEN, SpikeStyles.YELLOW, SpikeStyles.RED,
)

private const val WORST_ALPHABET =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ./:_-()[]{}<>=+*#@%&|~^\$?!,;'\"`\\"

class LineFactory(private val rng: Rng) {
    private val sb = StringBuilder(256)

    fun line(cols: Int, kind: Int, seq: Int): List<Run> = when (kind) {
        0 -> log(cols, seq)
        1 -> prompt(cols, seq)
        2 -> diff(cols, seq)
        3 -> code(cols, seq)
        4 -> progress(cols, seq)
        else -> boxed(cols, seq)
    }

    fun worstLine(cols: Int): List<Run> {
        val runs = ArrayList<Run>(12)
        var col = 0
        while (col < cols) {
            val len = (3 + rng.int(9)).coerceAtMost(cols - col)
            sb.setLength(0)
            repeat(len) { sb.append(WORST_ALPHABET[rng.int(WORST_ALPHABET.length)]) }
            runs.add(Run(rng.int(SpikeStyles.COUNT), sb.toString()))
            col += len
        }
        return runs
    }

    private fun words(n: Int): String {
        sb.setLength(0)
        repeat(n) {
            if (sb.isNotEmpty()) sb.append(' ')
            sb.append(WORDS[rng.int(WORDS.size)])
        }
        return sb.toString()
    }

    private fun log(cols: Int, seq: Int): List<Run> {
        val lvl = rng.int(LEVELS.size)
        val t = seq % 86400
        val stamp = two(t / 3600) + ":" + two(t / 60 % 60) + ":" + two(t % 60) + "." + three(seq % 1000)
        return listOf(
            Run(SpikeStyles.DIM, "$stamp "),
            Run(LEVEL_STYLES[lvl], LEVELS[lvl] + " "),
            Run(SpikeStyles.BLUE, PATHS[rng.int(PATHS.size)] + ": "),
            Run(SpikeStyles.DEFAULT, words(3 + rng.int(5)).take(maxOf(0, cols - 60))),
        )
    }

    private fun prompt(cols: Int, seq: Int): List<Run> = listOf(
        Run(SpikeStyles.BOLD_GREEN, "❯ "),
        Run(SpikeStyles.DEFAULT, "cargo "),
        Run(SpikeStyles.CYAN, "test "),
        Run(SpikeStyles.DEFAULT, "-p "),
        Run(SpikeStyles.MAGENTA, "kampr-term "),
        Run(SpikeStyles.ITALIC_DIM, "# $seq " + words(2)).let { it },
    )

    private fun diff(cols: Int, seq: Int): List<Run> {
        val add = seq % 2 == 0
        val st = if (add) SpikeStyles.DIFF_ADD else SpikeStyles.DIFF_DEL
        val body = (if (add) "+ " else "- ") + words(4 + rng.int(4))
        return listOf(Run(st, body.padEnd(cols).take(cols)))
    }

    private fun code(cols: Int, seq: Int): List<Run> = listOf(
        Run(SpikeStyles.DIM, four(seq % 9999) + " "),
        Run(SpikeStyles.MAGENTA, "fn "),
        Run(SpikeStyles.BOLD_BLUE, WORDS[rng.int(WORDS.size)]),
        Run(SpikeStyles.DEFAULT, "(buf: &mut "),
        Run(SpikeStyles.YELLOW, "CellBuffer"),
        Run(SpikeStyles.DEFAULT, ", n: "),
        Run(SpikeStyles.YELLOW, "usize"),
        Run(SpikeStyles.DEFAULT, ") -> "),
        Run(SpikeStyles.YELLOW, "Result"),
        Run(SpikeStyles.DEFAULT, "<()> {"),
    )

    private fun progress(cols: Int, seq: Int): List<Run> {
        val width = (cols - 20).coerceAtLeast(4)
        val filled = (seq * 3) % (width + 1)
        sb.setLength(0)
        repeat(filled) { sb.append('█') }
        val a = sb.toString()
        sb.setLength(0)
        repeat(width - filled) { sb.append('░') }
        return listOf(
            Run(SpikeStyles.DIM, "build "),
            Run(SpikeStyles.TRUECOLOR_BASE + (seq % SpikeStyles.TRUECOLOR_COUNT), a),
            Run(SpikeStyles.DIM, sb.toString()),
            Run(SpikeStyles.BOLD, " " + three(filled * 100 / width) + "%"),
        )
    }

    private fun boxed(cols: Int, seq: Int): List<Run> = listOf(
        Run(SpikeStyles.DIM, "│ "),
        Run(SpikeStyles.UNDERLINE_BLUE, "https://herdr.dev/docs/" + WORDS[rng.int(WORDS.size)]),
        Run(SpikeStyles.DEFAULT, "  "),
        Run(if (seq % 3 == 0) SpikeStyles.GREEN else SpikeStyles.RED, if (seq % 3 == 0) "✓" else "✗"),
        Run(SpikeStyles.DIM, " │"),
    )

    private fun two(v: Int) = if (v < 10) "0$v" else v.toString()
    private fun three(v: Int) = v.toString().padStart(3, '0')
    private fun four(v: Int) = v.toString().padStart(4, ' ')
}

class Workload(var profile: Profile, var cols: Int, var rows: Int) {
    private val rng = Rng()
    private val factory = LineFactory(rng)
    private var seq = 0
    private var rowAccumulator = 0.0
    private var msSinceReset = 0.0
    private var scrollRows = ArrayList<List<Run>>()

    var cursorCol = 0
        private set
    var cursorRow = 0
        private set

    fun reconfigure(profile: Profile, cols: Int, rows: Int) {
        this.profile = profile
        this.cols = cols
        this.rows = rows
        rng.reseed(0x51ED270Bu.toLong())
        seq = 0
        rowAccumulator = 0.0
        msSinceReset = 1e9
        scrollRows = ArrayList()
    }

    fun step(dtMs: Double, nowMs: Double): List<ServerMsg> {
        val out = ArrayList<ServerMsg>(2)
        val blink = ((nowMs / 530.0).toInt() % 2) == 0
        when (profile) {
            Profile.IDLE -> {
                cursorCol = (seq / 30) % cols
                out.add(GridPatch(emptyList(), CursorPos(cursorCol, cursorRow, blink)))
                seq++
            }

            Profile.MIXED -> {
                msSinceReset += dtMs
                if (msSinceReset >= 2000.0) {
                    msSinceReset = 0.0
                    out.add(fullReset(blink))
                } else {
                    rowAccumulator += 40.0 * dtMs / 1000.0
                    val n = rowAccumulator.toInt()
                    rowAccumulator -= n
                    if (n > 0) {
                        val diffs = ArrayList<RowDiff>(n)
                        repeat(n) {
                            val r = rng.int(rows)
                            diffs.add(RowDiff(r, factory.line(cols, seq % 6, seq)))
                            seq++
                        }
                        cursorCol = (cursorCol + 1) % cols
                        cursorRow = rows - 1
                        out.add(GridPatch(diffs, CursorPos(cursorCol, cursorRow, blink)))
                    } else {
                        out.add(GridPatch(emptyList(), CursorPos(cursorCol, cursorRow, blink)))
                    }
                }
            }

            Profile.SCROLL -> {
                if (scrollRows.size != rows) {
                    scrollRows = ArrayList(rows)
                    repeat(rows) { scrollRows.add(factory.line(cols, seq++ % 6, seq)) }
                }
                rowAccumulator += 30.0 * dtMs / 1000.0
                val n = rowAccumulator.toInt()
                rowAccumulator -= n
                if (n > 0) {
                    repeat(n) {
                        scrollRows.removeAt(0)
                        scrollRows.add(factory.line(cols, seq % 6, seq))
                        seq++
                    }
                    val diffs = ArrayList<RowDiff>(rows)
                    for (r in 0 until rows) diffs.add(RowDiff(r, scrollRows[r]))
                    out.add(GridPatch(diffs, CursorPos(cursorCol, rows - 1, blink)))
                } else {
                    out.add(GridPatch(emptyList(), CursorPos(cursorCol, rows - 1, blink)))
                }
            }

            Profile.WORST -> {
                val diffs = ArrayList<RowDiff>(rows)
                for (r in 0 until rows) diffs.add(RowDiff(r, factory.worstLine(cols)))
                seq++
                cursorCol = (cursorCol + 1) % cols
                out.add(GridPatch(diffs, CursorPos(cursorCol, rows - 1, blink)))
            }
        }
        return out
    }

    private fun fullReset(blink: Boolean): GridReset {
        val diffs = ArrayList<RowDiff>(rows)
        for (r in 0 until rows) {
            diffs.add(RowDiff(r, factory.line(cols, seq % 6, seq)))
            seq++
        }
        return GridReset(cols, rows, diffs, CursorPos(cursorCol, rows - 1, blink))
    }
}
