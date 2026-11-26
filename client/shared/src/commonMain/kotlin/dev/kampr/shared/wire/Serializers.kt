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
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
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

                is ColorSpec.Indexed -> buildJsonObject {
                    put("k", "i")
                    put("v", value.v)
                }

                is ColorSpec.Rgb -> buildJsonObject {
                    put("k", "r")
                    put("v", JsonArray(listOf(JsonPrimitive(value.r), JsonPrimitive(value.g), JsonPrimitive(value.b))))
                }
            },
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
                    put("b", "md")
                    put("text", value.text)
                    value.att?.let { put("att", attachmentJson(it)) }
                }

                is Block.Code -> buildJsonObject {
                    put("b", "code")
                    value.lang?.let { put("lang", it) }
                    put("text", value.text)
                    value.role?.let { put("role", it) }
                }

                is Block.Tool -> buildJsonObject {
                    put("b", "tool")
                    put("name", value.name)
                    value.summary?.let { put("summary", it) }
                    value.lines?.let { put("lines", it) }
                    value.state?.let { put("state", it) }
                }

                is Block.Diff -> buildJsonObject {
                    put("b", "diff")
                    value.path?.let { put("path", it) }
                    put("text", value.text)
                }

                is Block.Sub -> buildJsonObject {
                    put("b", "sub")
                    put("id", value.id)
                    value.kind?.let { put("kind", it) }
                    value.title?.let { put("title", it) }
                    value.depth?.let { put("depth", it) }
                }

                is Block.Unknown -> buildJsonObject { put("b", value.kind) }
            },
        )
    }
}

// serde spells the seam's answer the way it spells every externally tagged enum: a struct
// variant is {"allow": {...}}, and a unit variant is the bare word. The sealed hierarchy's own
// tags are a class discriminator in this library's version, which the node's serde does not
// decode, so the two sides are met in the middle here.
object ReplySerializer : KSerializer<Reply> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("kampr.Reply")

    override fun deserialize(decoder: Decoder): Reply {
        val el = (decoder as JsonDecoder).decodeJsonElement()
        val obj = el as? JsonObject ?: throw kotlinx.serialization.SerializationException("Unreadable reply $el")
        val entry = obj.entries.firstOrNull() ?: throw kotlinx.serialization.SerializationException("Empty reply")
        val body = entry.value as? JsonObject
        return when (entry.key) {
            "allow" -> Reply.Allow(
                input = body?.get("input"),
                remember = body?.get("remember")?.jsonPrimitive?.contentOrNull?.toBoolean() ?: false,
            )

            "deny" -> Reply.Deny(body?.get("message")?.jsonPrimitive?.contentOrNull)

            "choose" -> Reply.Choose((body?.get("answers") as? JsonArray)?.map { it.jsonPrimitive.content } ?: emptyList())

            "plan" -> {
                val o = body ?: throw kotlinx.serialization.SerializationException("Unreadable reply plan")
                Reply.Plan(
                    approve = o["approve"]?.jsonPrimitive?.contentOrNull?.toBoolean() ?: false,
                    mode = o["mode"]?.jsonPrimitive?.contentOrNull,
                    feedback = o["feedback"]?.jsonPrimitive?.contentOrNull,
                )
            }

            "form" -> Reply.Form(body?.get("values") ?: throw kotlinx.serialization.SerializationException("A reply form has no values"))

            else -> throw kotlinx.serialization.SerializationException("Unknown reply ${entry.key}")
        }
    }

    override fun serialize(encoder: Encoder, value: Reply) {
        val out = encoder as JsonEncoder
        out.encodeJsonElement(
            when (value) {
                is Reply.Allow -> buildJsonObject {
                    put(
                        "allow",
                        buildJsonObject {
                            value.input?.let { put("input", it) }
                            put("remember", value.remember)
                        },
                    )
                }

                is Reply.Deny -> buildJsonObject {
                    put("deny", buildJsonObject { value.message?.let { put("message", it) } })
                }

                is Reply.Choose -> buildJsonObject {
                    put(
                        "choose",
                        buildJsonObject {
                            put("answers", buildJsonArray { value.answers.forEach { add(JsonPrimitive(it)) } })
                        },
                    )
                }

                is Reply.Plan -> buildJsonObject {
                    put(
                        "plan",
                        buildJsonObject {
                            put("approve", value.approve)
                            value.mode?.let { put("mode", it) }
                            value.feedback?.let { put("feedback", it) }
                        },
                    )
                }

                is Reply.Form -> buildJsonObject {
                    put("form", buildJsonObject { put("values", value.values) })
                }

                is Reply.Dismiss -> JsonPrimitive("dismiss")
            },
        )
    }
}

// serde spells a unit variant as the bare string and a struct variant as the tagged object, so
// `login` goes out and comes back as "login" while `wait` arrives as {"wait":{"until":...}}. The
// sealed hierarchy's own tags would say {"login":null} for the units, which the node's serde does
// not decode, so the two sides are met in the middle here.
object TroubleFixSerializer : KSerializer<TroubleFix> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("kampr.TroubleFix")

    override fun deserialize(decoder: Decoder): TroubleFix {
        val el = (decoder as JsonDecoder).decodeJsonElement()
        return when {
            el is JsonPrimitive && el.isString -> when (el.content) {
                "login" -> TroubleFix.Login
                "billing" -> TroubleFix.Billing
                else -> throw kotlinx.serialization.SerializationException("Unknown trouble fix ${el.content}")
            }

            el is JsonObject -> {
                val entry = el.entries.firstOrNull() ?: throw kotlinx.serialization.SerializationException("Empty trouble fix")
                if (entry.key != "wait") throw kotlinx.serialization.SerializationException("Unknown trouble fix ${entry.key}")
                TroubleFix.Wait(entry.value?.jsonObject?.get("until")?.jsonPrimitive?.contentOrNull.orEmpty())
            }

            else -> throw kotlinx.serialization.SerializationException("Unreadable trouble fix $el")
        }
    }

    override fun serialize(encoder: Encoder, value: TroubleFix) {
        val out = encoder as JsonEncoder
        out.encodeJsonElement(
            when (value) {
                is TroubleFix.Login -> JsonPrimitive("login")
                is TroubleFix.Billing -> JsonPrimitive("billing")
                is TroubleFix.Wait -> buildJsonObject { put("wait", buildJsonObject { put("until", value.until) }) }
            },
        )
    }
}
