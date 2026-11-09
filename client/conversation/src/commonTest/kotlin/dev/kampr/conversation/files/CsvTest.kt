package dev.kampr.conversation.files

import kotlin.test.Test
import kotlin.test.assertEquals

private fun parsed(text: String, delimiter: Char = ',', batches: Int = 1): List<List<String>> {
    val lines = text.split('\n')
    val records = CsvRecords(delimiter)
    val step = maxOf(1, lines.size / batches)
    var fed = 0
    while (fed < lines.size) {
        fed = minOf(lines.size, fed + step)
        records.feed(lines.subList(0, fed), ended = fed == lines.size)
    }
    return records.rows
}

class CsvTest {
    @Test
    fun recordsAreSplitTheWayASpreadsheetWritesThem() {
        val table = listOf(
            "plain" to ("a,b,c\n1,2,3" to listOf(listOf("a", "b", "c"), listOf("1", "2", "3"))),
            "crlf" to ("a,b\r\n1,2\r" to listOf(listOf("a", "b"), listOf("1", "2"))),
            "quoted comma" to ("name,city\n\"Smith, J\",Perth" to listOf(listOf("name", "city"), listOf("Smith, J", "Perth"))),
            "doubled quote" to ("q\n\"he said \"\"hi\"\"\"" to listOf(listOf("q"), listOf("he said \"hi\""))),
            "newline inside quotes" to ("note,n\n\"two\nlines\",2\nnext,3" to listOf(listOf("note", "n"), listOf("two\nlines", "2"), listOf("next", "3"))),
            "empty fields" to ("a,,c,\n,,," to listOf(listOf("a", "", "c", ""), listOf("", "", "", ""))),
            "trailing newline is not a row" to ("a\n1\n" to listOf(listOf("a"), listOf("1"))),
            "blank line inside is skipped" to ("a\n\n1" to listOf(listOf("a"), listOf("1"))),
            "unterminated quote keeps what it has" to ("a\n\"open,1" to listOf(listOf("a"), listOf("open,1"))),
        )
        for ((named, case) in table) assertEquals(case.second, parsed(case.first), named)
    }

    @Test
    fun aRecordSplitAcrossSlicesIsTheSameRecord() {
        val text = "id,body\n1,\"first\nsecond\nthird\"\n2,\"x, y\"\n3,z"
        val whole = parsed(text)
        for (batches in 2..7) assertEquals(whole, parsed(text, batches = batches), "$batches batches")
    }

    @Test
    fun tabsSeparateATsvAndQuotesStillGroup() {
        assertEquals(listOf(listOf("a", "b c"), listOf("1", "x\ty")), parsed("a\tb c\n1\t\"x\ty\"", '\t'))
    }

    @Test
    fun theSeparatorIsWhicheverTheHeaderUses() {
        assertEquals(',', csvDelimiter("a.csv", "name,qty,price"))
        assertEquals(';', csvDelimiter("a.csv", "name;qty;\"price, incl\""))
        assertEquals('\t', csvDelimiter("a.tsv", "name\tqty"))
        assertEquals(null, csvDelimiter("a.txt", "name,qty"))
    }
}
