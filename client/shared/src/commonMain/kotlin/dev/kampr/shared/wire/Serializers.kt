package dev.kampr.shared.wire

import kotlinx.serialization.KSerializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

object ColorSpecSerializer : KSerializer<ColorSpec> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("kampr.ColorSpec")

    override fun deserialize(decoder: Decoder): ColorSpec {
        val obj = (decoder as JsonDecoder).decodeJsonElement() as? JsonObject ?: return ColorSpec.Default
        return when (obj["k"]?.jsonPrimitive?.contentOrNull) {
            "i" -> ColorSpec.Indexed(obj["v"]?.jsonPrimitive?.intOrNull ?: 0)
            "r" -> {
                val v = obj["v"] as? JsonArray ?: return ColorSpec.Default
                ColorSpec.Rgb(
                    v.getOrNull(0)?.jsonPrimitive?.intOrNull ?: 0,
                    v.getOrNull(1)?.jsonPrimitive?.intOrNull ?: 0,
                    v.getOrNull(2)?.jsonPrimitive?.intOrNull ?: 0,
                )
            }
            else -> ColorSpec.Default
        }
    }

    override fun serialize(encoder: Encoder, value: ColorSpec) {
        val out = encoder as JsonEncoder
        out.encodeJsonElement(
            when (value) {
                is ColorSpec.Default -> buildJsonObject { put("k", "d") }
                is ColorSpec.Indexed -> buildJsonObject { put("k", "i"); put("v", value.v) }
                is ColorSpec.Rgb -> buildJsonObject {
                    put("k", "r")
                    put("v", JsonArray(listOf(JsonPrimitive(value.r), JsonPrimitive(value.g), JsonPrimitive(value.b))))
                }
            }
        )
    }
}

object BlockSerializer : KSerializer<Block> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("kampr.Block")

    override fun deserialize(decoder: Decoder): Block {
        val obj = (decoder as JsonDecoder).decodeJsonElement() as? JsonObject
            ?: return Block.Unknown("")
        val kind = obj["b"]?.jsonPrimitive?.contentOrNull ?: ""
        val text = obj["text"]?.jsonPrimitive?.contentOrNull ?: ""
        return when (kind) {
            "md" -> Block.Md(text, attachmentOf(obj["att"]))
            "code" -> Block.Code(
                obj["lang"]?.jsonPrimitive?.contentOrNull,
                text,
                obj["role"]?.jsonPrimitive?.contentOrNull,
            )
            "tool" -> Block.Tool(
                name = obj["name"]?.jsonPrimitive?.contentOrNull ?: "tool",
                summary = obj["summary"]?.jsonPrimitive?.contentOrNull,
                lines = obj["lines"]?.jsonPrimitive?.intOrNull,
                state = obj["state"]?.jsonPrimitive?.contentOrNull,
            )
            "diff" -> Block.Diff(obj["path"]?.jsonPrimitive?.contentOrNull, text)
            // Without an id there is nothing to hand back, and a card offering to open a
            // conversation it cannot name is the inert affordance rule in miniature.
            "sub" -> (obj["id"] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotEmpty() }?.let { id ->
                Block.Sub(
                    id = id,
                    kind = (obj["kind"] as? JsonPrimitive)?.contentOrNull,
                    title = (obj["title"] as? JsonPrimitive)?.contentOrNull,
                    depth = (obj["depth"] as? JsonPrimitive)?.intOrNull,
                )
            } ?: Block.Unknown(kind)
            else -> Block.Unknown(kind)
        }
    }

    // Read field by field rather than through the generated decoder: an `att` carrying a field
    // this release has never heard of, or a known field of the wrong shape, must still hand over
    // the id it does carry instead of taking the whole turn down with it.
    private fun attachmentOf(element: JsonElement?): Attachment? {
        val obj = element as? JsonObject ?: return null
        val id = (obj["id"] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotEmpty() } ?: return null
        return Attachment(
            id = id,
            kind = (obj["kind"] as? JsonPrimitive)?.contentOrNull.orEmpty(),
            mime = (obj["mime"] as? JsonPrimitive)?.contentOrNull,
            bytes = (obj["bytes"] as? JsonPrimitive)?.longOrNull,
            name = (obj["name"] as? JsonPrimitive)?.contentOrNull,
        )
    }

    private fun attachmentJson(att: Attachment): JsonObject = buildJsonObject {
        put("id", att.id)
        if (att.kind.isNotEmpty()) put("kind", att.kind)
        att.mime?.let { put("mime", it) }
        att.bytes?.let { put("bytes", it) }
        att.name?.let { put("name", it) }
    }

    override fun serialize(encoder: Encoder, value: Block) {
        val out = encoder as JsonEncoder
        out.encodeJsonElement(
            when (value) {
                is Block.Md -> buildJsonObject {
                    put("b", "md"); put("text", value.text)
                    value.att?.let { put("att", attachmentJson(it)) }
                }
                is Block.Code -> buildJsonObject {
                    put("b", "code"); value.lang?.let { put("lang", it) }; put("text", value.text)
                    value.role?.let { put("role", it) }
                }
                is Block.Tool -> buildJsonObject {
                    put("b", "tool"); put("name", value.name)
                    value.summary?.let { put("summary", it) }
                    value.lines?.let { put("lines", it) }
                    value.state?.let { put("state", it) }
                }
                is Block.Diff -> buildJsonObject { put("b", "diff"); value.path?.let { put("path", it) }; put("text", value.text) }
                is Block.Sub -> buildJsonObject {
                    put("b", "sub"); put("id", value.id)
                    value.kind?.let { put("kind", it) }
                    value.title?.let { put("title", it) }
                    value.depth?.let { put("depth", it) }
                }
                is Block.Unknown -> buildJsonObject { put("b", value.kind) }
            }
        )
    }
}
