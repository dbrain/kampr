package dev.kampr.shared.util

// Byte mode, error correction M, versions 1 to 10 — 213 bytes, which is an origin and a pairing
// code several times over. A dependency for this would be a whole multiplatform artefact for one
// picture of one short URL, and there is no decoder here to keep in step with it.
class QrCode(val size: Int, private val modules: BooleanArray) {
    fun dark(x: Int, y: Int): Boolean = modules[y * size + x]
}

private const val MAX_VERSION = 10

// Error correction level M. Index is the version; index 0 is unused.
private val ECC_PER_BLOCK = intArrayOf(0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26)
private val BLOCKS = intArrayOf(0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5)

private val ALIGNMENT = arrayOf(
    intArrayOf(),
    intArrayOf(),
    intArrayOf(6, 18),
    intArrayOf(6, 22),
    intArrayOf(6, 26),
    intArrayOf(6, 30),
    intArrayOf(6, 34),
    intArrayOf(6, 22, 38),
    intArrayOf(6, 24, 42),
    intArrayOf(6, 26, 46),
    intArrayOf(6, 28, 50),
)

// The modules a version has left for data once every function pattern is subtracted, including
// the remainder bits that are placed but never carry a codeword.
private fun rawDataModules(version: Int): Int {
    var result = (16 * version + 128) * version + 64
    if (version >= 2) {
        val numAlign = version / 7 + 2
        result -= (25 * numAlign - 10) * numAlign - 55
        if (version >= 7) result -= 36
    }
    return result
}

private fun dataCapacityBits(version: Int): Int =
    rawDataModules(version) / 8 * 8 - ECC_PER_BLOCK[version] * BLOCKS[version] * 8

private fun countBits(version: Int): Int = if (version <= 9) 8 else 16

fun qrEncode(text: String): QrCode? {
    val bytes = text.encodeToByteArray()
    val version = (1..MAX_VERSION).firstOrNull {
        dataCapacityBits(it) >= 4 + countBits(it) + bytes.size * 8
    } ?: return null
    return QrBuilder(version, bytes).build()
}

// The origin plus, when there is one, the pairing code — in the fragment, which is never sent to
// the node and so never lands in its access log or the reverse proxy's.
fun joinLink(origin: String, code: String?): String {
    val base = origin.trimEnd('/')
    return if (code.isNullOrBlank()) base else "$base#pair=${code.trim()}"
}

private class QrBuilder(private val version: Int, private val data: ByteArray) {
    private val size = version * 4 + 17
    private val modules = BooleanArray(size * size)
    private val function = BooleanArray(size * size)

    fun build(): QrCode {
        drawFunctionPatterns()
        drawCodewords(withEcc(payload()))
        var best = 0
        var bestScore = Int.MAX_VALUE
        for (mask in 0..7) {
            applyMask(mask)
            drawFormat(mask)
            val score = penalty()
            if (score < bestScore) {
                bestScore = score
                best = mask
            }
            applyMask(mask)
        }
        applyMask(best)
        drawFormat(best)
        return QrCode(size, modules)
    }

    private fun at(x: Int, y: Int) = y * size + x

    private fun setFunction(x: Int, y: Int, dark: Boolean) {
        if (x !in 0 until size || y !in 0 until size) return
        modules[at(x, y)] = dark
        function[at(x, y)] = true
    }

    private fun drawFunctionPatterns() {
        for (i in 0 until size) {
            setFunction(6, i, i % 2 == 0)
            setFunction(i, 6, i % 2 == 0)
        }
        finder(3, 3)
        finder(size - 4, 3)
        finder(3, size - 4)
        val positions = ALIGNMENT[version]
        for (i in positions.indices) {
            for (j in positions.indices) {
                val corner = (i == 0 && j == 0) ||
                    (i == 0 && j == positions.size - 1) ||
                    (i == positions.size - 1 && j == 0)
                if (!corner) alignment(positions[i], positions[j])
            }
        }
        drawFormat(0)
        drawVersion()
    }

    private fun finder(x: Int, y: Int) {
        for (dy in -4..4) {
            for (dx in -4..4) {
                val dist = maxOf(abs(dx), abs(dy))
                setFunction(x + dx, y + dy, dist != 2 && dist != 4)
            }
        }
    }

    private fun alignment(x: Int, y: Int) {
        for (dy in -2..2) {
            for (dx in -2..2) {
                setFunction(x + dx, y + dy, maxOf(abs(dx), abs(dy)) != 1)
            }
        }
    }

    private fun drawFormat(mask: Int) {
        // Level M is 0b00, and the BCH remainder plus the fixed mask keeps the 15 bits far apart
        // from every other level's.
        val value = mask
        var rem = value
        repeat(10) { rem = (rem shl 1) xor ((rem ushr 9) * 0x537) }
        val bits = ((value shl 10) or rem) xor 0x5412
        for (i in 0..5) setFunction(8, i, bit(bits, i))
        setFunction(8, 7, bit(bits, 6))
        setFunction(8, 8, bit(bits, 7))
        setFunction(7, 8, bit(bits, 8))
        for (i in 9..14) setFunction(14 - i, 8, bit(bits, i))
        for (i in 0..7) setFunction(size - 1 - i, 8, bit(bits, i))
        for (i in 8..14) setFunction(8, size - 15 + i, bit(bits, i))
        setFunction(8, size - 8, true)
    }

    private fun drawVersion() {
        if (version < 7) return
        var rem = version
        repeat(12) { rem = (rem shl 1) xor ((rem ushr 11) * 0x1F25) }
        val bits = (version shl 12) or rem
        for (i in 0 until 18) {
            val dark = bit(bits, i)
            val a = size - 11 + i % 3
            val b = i / 3
            setFunction(a, b, dark)
            setFunction(b, a, dark)
        }
    }

    private fun payload(): ByteArray {
        val capacity = dataCapacityBits(version)
        val bits = BitBuffer()
        bits.append(4, 4)
        bits.append(data.size, countBits(version))
        for (b in data) bits.append(b.toInt() and 0xFF, 8)
        bits.append(0, minOf(4, capacity - bits.length))
        bits.append(0, (8 - bits.length % 8) % 8)
        var pad = 0xEC
        while (bits.length < capacity) {
            bits.append(pad, 8)
            pad = pad xor 0xEC xor 0x11
        }
        return bits.bytes()
    }

    // Blocks are interleaved codeword by codeword, so a scratch across the symbol damages a few
    // codewords of every block rather than destroying one outright.
    private fun withEcc(codewords: ByteArray): ByteArray {
        val blocks = BLOCKS[version]
        val eccLen = ECC_PER_BLOCK[version]
        val raw = rawDataModules(version) / 8
        val shortBlocks = blocks - raw % blocks
        val shortLen = raw / blocks
        val divisor = rsDivisor(eccLen)
        val built = ArrayList<ByteArray>(blocks)
        var read = 0
        for (i in 0 until blocks) {
            val length = shortLen - eccLen + (if (i < shortBlocks) 0 else 1)
            val chunk = codewords.copyOfRange(read, read + length)
            read += length
            val block = chunk.copyOf(shortLen + 1)
            rsRemainder(chunk, divisor).copyInto(block, block.size - eccLen)
            built += block
        }
        val result = ByteArray(raw)
        var write = 0
        for (i in 0 until shortLen + 1) {
            for (j in built.indices) {
                if (i != shortLen - eccLen || j >= shortBlocks) {
                    result[write] = built[j][i]
                    write++
                }
            }
        }
        return result
    }

    private fun drawCodewords(codewords: ByteArray) {
        var i = 0
        var right = size - 1
        while (right >= 1) {
            if (right == 6) right = 5
            for (vert in 0 until size) {
                for (j in 0..1) {
                    val x = right - j
                    val upward = ((right + 1) and 2) == 0
                    val y = if (upward) size - 1 - vert else vert
                    if (!function[at(x, y)] && i < codewords.size * 8) {
                        modules[at(x, y)] = bit(codewords[i ushr 3].toInt(), 7 - (i and 7))
                        i++
                    }
                }
            }
            right -= 2
        }
    }

    private fun applyMask(mask: Int) {
        for (y in 0 until size) {
            for (x in 0 until size) {
                if (function[at(x, y)]) continue
                val invert = when (mask) {
                    0 -> (x + y) % 2 == 0
                    1 -> y % 2 == 0
                    2 -> x % 3 == 0
                    3 -> (x + y) % 3 == 0
                    4 -> (x / 3 + y / 2) % 2 == 0
                    5 -> x * y % 2 + x * y % 3 == 0
                    6 -> (x * y % 2 + x * y % 3) % 2 == 0
                    else -> ((x + y) % 2 + x * y % 3) % 2 == 0
                }
                if (invert) modules[at(x, y)] = !modules[at(x, y)]
            }
        }
    }

    private fun penalty(): Int {
        var result = 0
        for (y in 0 until size) result += lineScore { x -> modules[at(x, y)] }
        for (x in 0 until size) result += lineScore { y -> modules[at(x, y)] }
        for (y in 0 until size - 1) {
            for (x in 0 until size - 1) {
                val c = modules[at(x, y)]
                if (c == modules[at(x + 1, y)] && c == modules[at(x, y + 1)] && c == modules[at(x + 1, y + 1)]) {
                    result += 3
                }
            }
        }
        val dark = modules.count { it }
        val total = size * size
        val k = (abs(dark * 20 - total * 10) + total - 1) / total - 1
        return result + k * 10
    }

    private inline fun lineScore(cell: (Int) -> Boolean): Int {
        var result = 0
        var colour = false
        var run = 0
        val history = IntArray(7)
        for (i in 0 until size) {
            if (cell(i) == colour) {
                run++
                if (run == 5) result += 3 else if (run > 5) result++
            } else {
                pushRun(run, history)
                if (!colour) result += finderLike(history) * 40
                colour = cell(i)
                run = 1
            }
        }
        if (colour) {
            pushRun(run, history)
            run = 0
        }
        pushRun(run + size, history)
        return result + finderLike(history) * 40
    }

    private fun pushRun(run: Int, history: IntArray) {
        val length = if (history[0] == 0) run + size else run
        history.copyInto(history, 1, 0, history.size - 1)
        history[0] = length
    }

    // The 1:1:3:1:1 ratio a decoder reads as a finder pattern. One of these in the data area is
    // what makes a symbol scan as the wrong thing, or not at all.
    private fun finderLike(history: IntArray): Int {
        val n = history[1]
        val core = n > 0 && history[2] == n && history[3] == n * 3 && history[4] == n && history[5] == n
        if (!core) return 0
        var count = 0
        if (history[0] >= n * 4 && history[6] >= n) count++
        if (history[6] >= n * 4 && history[0] >= n) count++
        return count
    }
}

private class BitBuffer {
    private val bits = ArrayList<Boolean>()
    val length: Int get() = bits.size

    fun append(value: Int, count: Int) {
        for (i in count - 1 downTo 0) bits += ((value ushr i) and 1) != 0
    }

    fun bytes(): ByteArray {
        val out = ByteArray((bits.size + 7) / 8)
        bits.forEachIndexed { i, on -> if (on) out[i ushr 3] = (out[i ushr 3].toInt() or (1 shl (7 - (i and 7)))).toByte() }
        return out
    }
}

private fun rsDivisor(degree: Int): ByteArray {
    val result = ByteArray(degree)
    result[degree - 1] = 1
    var root = 1
    for (i in 0 until degree) {
        for (j in 0 until degree) {
            result[j] = rsMultiply(result[j].toInt() and 0xFF, root).toByte()
            if (j + 1 < degree) result[j] = (result[j].toInt() xor result[j + 1].toInt()).toByte()
        }
        root = rsMultiply(root, 0x02)
    }
    return result
}

private fun rsRemainder(data: ByteArray, divisor: ByteArray): ByteArray {
    val result = ByteArray(divisor.size)
    for (b in data) {
        val factor = (b.toInt() xor result[0].toInt()) and 0xFF
        result.copyInto(result, 0, 1, result.size)
        result[result.size - 1] = 0
        for (i in result.indices) {
            result[i] = (result[i].toInt() xor rsMultiply(divisor[i].toInt() and 0xFF, factor)).toByte()
        }
    }
    return result
}

private fun rsMultiply(x: Int, y: Int): Int {
    var z = 0
    for (i in 7 downTo 0) {
        z = (z shl 1) xor ((z ushr 7) * 0x11D)
        z = z xor (((y ushr i) and 1) * x)
    }
    return z and 0xFF
}

private fun bit(value: Int, index: Int): Boolean = ((value ushr index) and 1) != 0

private fun abs(value: Int): Int = if (value < 0) -value else value
