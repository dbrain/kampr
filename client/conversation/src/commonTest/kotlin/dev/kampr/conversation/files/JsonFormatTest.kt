package dev.kampr.conversation.files

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class JsonFormatTest {
    @Test
    fun jsonIsLaidOutTwoSpacesALevel() {
        val table = listOf(
            "object and array" to ("""{"a":1,"b":[true,null]}""" to "{\n  \"a\": 1,\n  \"b\": [\n    true,\n    null\n  ]\n}"),
            "empty containers stay on their line" to ("""{"a":[],"b":{ }}""" to "{\n  \"a\": [],\n  \"b\": {}\n}"),
            "strings keep their escapes and brackets" to ("""["a\"b,{c}:[","\\"]""" to "[\n  \"a\\\"b,{c}:[\",\n  \"\\\\\"\n]"),
            "whitespace between scalars is kept as one space" to ("[1 \n 2]" to "[\n  1 2\n]"),
            "scalar alone" to (" 42 " to "42"),
            "truncated mid-array" to ("""{"a":[1,2""" to "{\n  \"a\": [\n    1,\n    2"),
            "unterminated string" to ("""{"a":"abc""" to "{\n  \"a\": \"abc"),
            "a closer with nothing open" to ("]}" to "]\n}"),
            "trailing comma" to ("[1,]" to "[\n  1,\n]"),
            "line comment ends its line" to ("{// why\n\"a\":1 /* b */}" to "{\n  // why\n  \"a\": 1 /* b */\n}"),
            "empty" to ("" to ""),
        )
        for ((named, case) in table) assertEquals(case.second, formatJson(case.first), named)
    }

    @Test
    fun formattingTwiceChangesNothing() {
        val once = formatJson("""{"x":[{"y":"z, w"},[],{}],"n":-1.5e3,"s":"é"}""")
        assertEquals(once, formatJson(once))
    }

    @Test
    fun deepNestingIsBoundedNotQuadratic() {
        val depth = 5_000
        val out = formatJson("[".repeat(depth) + "]".repeat(depth))
        assertTrue(out.length < depth * 200, "${out.length} characters")
    }
}
