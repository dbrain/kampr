package dev.kampr.conversation.md

enum class Align { Start, Center, End }

sealed interface MdBlock {
    data class Heading(val level: Int, val text: String) : MdBlock
    data class Paragraph(val text: String) : MdBlock
    data class Fence(val lang: String?, val code: String) : MdBlock
    data class Quote(val blocks: List<MdBlock>) : MdBlock
    data class Table(
        val header: List<String>,
        val rows: List<List<String>>,
        val aligns: List<Align>,
    ) : MdBlock
    data class Bullets(val items: List<MdItem>, val ordered: Boolean) : MdBlock
    data object Rule : MdBlock
}

data class MdItem(val marker: String, val blocks: List<MdBlock>)
