package dev.kampr.conversation.files

// A spreadsheet in some locales writes `;` where a comma is the decimal point, and names the file
// `.csv` all the same; whichever of the two the header uses more of, outside quotes, is the one.
fun csvDelimiter(name: String, header: String): Char? = when (name.substringAfterLast('.', "").lowercase()) {
    "tsv", "tab" -> '\t'

    "csv" -> {
        var quoted = false
        var commas = 0
        var semicolons = 0
        for (c in header) {
            when {
                c == '"' -> quoted = !quoted
                quoted -> Unit
                c == ',' -> commas++
                c == ';' -> semicolons++
            }
        }
        if (semicolons > commas) ';' else ','
    }

    else -> null
}

// RFC 4180 records out of lines that arrive a slice at a time. A quoted field may hold the
// separator, a doubled quote and newlines, so a record can span lines and the slices between them;
// what an open quote has gathered waits for the next lines rather than being cut into rows.
class CsvRecords(private val delimiter: Char) {
    private val done = ArrayList<List<String>>()
    private var consumed = 0
    private var fields = ArrayList<String>()
    private val field = StringBuilder()
    private var quoted = false
    private var open = false

    val rows: List<List<String>> get() = done

    fun feed(lines: List<String>, ended: Boolean) {
        while (consumed < lines.size) {
            val line = lines[consumed++].removeSuffix("\r")
            if (!open && line.isEmpty()) continue
            if (open) field.append('\n')
            read(line)
        }
        if (ended && open) {
            quoted = false
            close()
        }
    }

    private fun read(line: String) {
        open = true
        var i = 0
        while (i < line.length) {
            val c = line[i]
            when {
                quoted && c == '"' && line.getOrNull(i + 1) == '"' -> {
                    field.append('"')
                    i++
                }

                quoted && c == '"' -> quoted = false

                quoted -> field.append(c)

                c == '"' && field.isEmpty() -> quoted = true

                c == delimiter -> {
                    fields += field.toString()
                    field.clear()
                }

                else -> field.append(c)
            }
            i++
        }
        if (!quoted) close()
    }

    private fun close() {
        fields += field.toString()
        field.clear()
        done += fields
        fields = ArrayList()
        open = false
    }
}
