package dev.kampr.shared

import dev.kampr.shared.util.joinLink
import dev.kampr.shared.util.qrEncode
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

// The symbol `QrDecodeTest` handed to zbar, module for module. That test only runs on the JVM
// where a decoder is installed; this is what says the same encoder produces the same symbol on
// wasm and on Android, where the arithmetic is not the same arithmetic.
private val GOLDEN = listOf(
    "#######..#..##.#.#.#..#######",
    "#.....#....######.#.#.#.....#",
    "#.###.#.#.#.##.##..##.#.###.#",
    "#.###.#.####..#..##.#.#.###.#",
    "#.###.#.#..#...##..#..#.###.#",
    "#.....#.#####..#..#...#.....#",
    "#######.#.#.#.#.#.#.#.#######",
    "........####..#...#..........",
    "#.#####...###..######.#####..",
    "..#..#.#..#..##...##..#.#...#",
    "###...#.#..#####.##....##....",
    "##.#.#...##..#..#..#.##.#..#.",
    ".##########.#.##.#.###.#.##..",
    "#..#...#...##..##..#..#.#.#.#",
    "###...##.#.##..##.#..##...#..",
    "###.##....#.#.##..#.##.#...#.",
    "..#####..####..###..##....#..",
    "###.....#...#####.##.##.###.#",
    "#.#####..########.#...#..##..",
    "#....#.##.#.##..#..#.##.#..#.",
    "#.#..###....#.####..#####.###",
    "........#.#.#....####...#####",
    "#######...#....##.###.#.###..",
    "#.....#.####..#.....#...#..#.",
    "#.###.#.###.#....#..#######..",
    "#.###.#.##.##.###...##.#...##",
    "#.###.#.##.##..######.#.##.#.",
    "#.....#...#...###.#.##.##..#.",
    "#######.##.##.#.###......##..",
)

class QrGoldenTest {
    @Test
    fun everyPlatformDrawsTheSymbolZbarRead() {
        val code = assertNotNull(qrEncode(joinLink("http://192.168.1.24:8790", "K7QF2M")))
        assertEquals(GOLDEN.size, code.size)
        val drawn = (0 until code.size).map { y ->
            (0 until code.size).joinToString("") { x -> if (code.dark(x, y)) "#" else "." }
        }
        assertEquals(GOLDEN, drawn)
    }
}
