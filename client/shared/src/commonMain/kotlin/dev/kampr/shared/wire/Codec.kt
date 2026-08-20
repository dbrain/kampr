package dev.kampr.shared.wire

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

object Wire {
    val json: Json = Json {
        ignoreUnknownKeys = true
        isLenient = true
        explicitNulls = false
        encodeDefaults = false
        coerceInputValues = true
    }

    fun decode(text: String): ServerMsg? = runCatching { decodeElement(json.parseToJsonElement(text)) }.getOrNull()

    fun decodeElement(element: JsonElement): ServerMsg? {
        val obj = element as? JsonObject ?: return null
        return when (obj.str("t")) {
            "hello" -> ServerMsg.Hello(
                protocol = obj.int("protocol") ?: 1,
                nodeId = obj.str("node_id").orEmpty(),
                nodeName = obj.str("node_name").orEmpty(),
                build = obj.str("build").orEmpty(),
                role = obj.str("role") ?: "full",
                caps = obj.decode("caps") ?: Caps(),
                security = obj.decode("security") ?: Security(),
            )
            "herd" -> ServerMsg.Herd(
                nodes = obj.decodeList<NodeInfo>("nodes"),
                panes = obj.decodeList<PaneInfo>("panes"),
            )
            "herd.patch" -> ServerMsg.HerdPatch(
                added = obj.decode<HerdDelta>("added") ?: HerdDelta(),
                changed = obj.decode<HerdDelta>("changed") ?: HerdDelta(),
                removedIds = obj.strings("removed_ids"),
            )
            "styles" -> ServerMsg.Styles(
                from = obj.int("from") ?: 0,
                styles = obj.decodeList<Style>("styles"),
            )
            "grid.reset" -> ServerMsg.GridReset(
                pane = obj.str("pane") ?: return null,
                cols = obj.int("cols") ?: return null,
                rows = obj.int("rows") ?: return null,
                rowsData = obj.decodeList<RowDiff>("rows_data"),
                cursor = obj.decode<Cursor>("cursor") ?: Cursor(),
                links = obj.strings("links"),
            )
            "grid.patch" -> ServerMsg.GridPatch(
                pane = obj.str("pane") ?: return null,
                rows = obj.decodeList<RowDiff>("rows"),
                cursor = obj.decode<Cursor>("cursor"),
                links = obj.strings("links"),
            )
            "scrollback" -> ServerMsg.Scrollback(
                pane = obj.str("pane") ?: return null,
                fromTop = obj.int("from_top") ?: 0,
                rows = obj.decodeList<RowDiff>("rows"),
                totalRows = obj.int("total_rows") ?: 0,
                complete = obj.bool("complete") ?: false,
                capped = obj.bool("capped") ?: false,
            )
            "convo" -> ServerMsg.Convo(
                pane = obj.str("pane") ?: return null,
                cursor = obj.str("cursor"),
                more = obj.bool("more") ?: false,
                turns = obj.decodeList<Turn>("turns"),
            )
            "convo.turn" -> ServerMsg.ConvoTurn(
                pane = obj.str("pane") ?: return null,
                turns = obj.decodeList<Turn>("turns").ifEmpty { listOfNotNull(obj.decode<Turn>("turn")) },
            )
            "pending" -> ServerMsg.Pending(
                pane = obj.str("pane") ?: return null,
                question = obj.str("question")?.takeIf { it.isNotBlank() },
                options = obj.decodeList<PendingOption>("options"),
                source = obj.str("source") ?: "screen",
            )
            "managed" -> ServerMsg.Managed(
                op = obj.str("op").orEmpty(),
                ok = obj.bool("ok") ?: false,
                id = obj.str("id"),
            )
            "caps" -> ServerMsg.NodeCaps(
                node = obj.str("node").orEmpty(),
                agentKinds = obj.strings("agent_kinds"),
                sessions = obj.decodeList<SessionInfo>("sessions"),
            )
            "error" -> ServerMsg.Failure(
                code = obj.str("code") ?: "bad_request",
                message = obj.str("message").orEmpty(),
                pane = obj.str("pane"),
            )
            "prefs" -> ServerMsg.Prefs(
                panes = (obj["panes"] as? JsonObject)?.mapValues { (_, value) ->
                    PanePrefs(
                        (value as? JsonObject)?.mapNotNull { (key, item) ->
                            (item as? JsonPrimitive)?.contentOrNull?.let { key to it }
                        }?.toMap() ?: emptyMap()
                    )
                } ?: emptyMap()
            )
            "pong" -> ServerMsg.Pong(obj.int("n") ?: 0)
            else -> null
        }
    }

    fun encode(msg: ClientMsg): String = json.encodeToString(JsonObject.serializer(), element(msg))

    private fun element(msg: ClientMsg): JsonObject = when (msg) {
        is ClientMsg.Watch -> buildJsonObject {
            put("t", "watch"); put("pane", msg.pane)
            put("scrollback", msg.scrollback); put("conversation", msg.conversation)
        }
        is ClientMsg.Unwatch -> buildJsonObject { put("t", "unwatch"); put("pane", msg.pane) }
        is ClientMsg.InputText -> buildJsonObject { put("t", "input"); put("pane", msg.pane); put("text", msg.text) }
        is ClientMsg.InputB64 -> buildJsonObject { put("t", "input"); put("pane", msg.pane); put("b64", msg.b64) }
        is ClientMsg.InputKeys -> buildJsonObject {
            put("t", "input"); put("pane", msg.pane)
            put("keys", buildJsonArray { msg.keys.forEach { add(JsonPrimitive(it)) } })
        }
        is ClientMsg.Answer -> buildJsonObject { put("t", "answer"); put("pane", msg.pane); put("key", msg.key) }
        is ClientMsg.ConvoLoad -> buildJsonObject {
            put("t", "convo.load"); put("pane", msg.pane); msg.before?.let { put("before", it) }
        }
        is ClientMsg.SetPrefs -> buildJsonObject {
            put("t", "prefs"); put("pane", msg.pane)
            put("prefs", buildJsonObject { msg.prefs.forEach { (k, v) -> put(k, v) } })
        }
        ClientMsg.Resync -> buildJsonObject { put("t", "resync") }
        is ClientMsg.Ping -> buildJsonObject { put("t", "ping"); put("n", msg.n) }
        is ClientMsg.Manage -> buildJsonObject {
            put("t", "manage"); put("op", msg.op)
            msg.fields.forEach { (k, v) -> if (v != null) put(k, v) }
        }
    }

    private fun JsonObject.prim(key: String): JsonPrimitive? = (this[key] as? JsonPrimitive)

    private fun JsonObject.str(key: String): String? = prim(key)?.contentOrNull

    private fun JsonObject.int(key: String): Int? = prim(key)?.intOrNull

    private fun JsonObject.bool(key: String): Boolean? = prim(key)?.booleanOrNull

    private fun JsonObject.strings(key: String): List<String> =
        (this[key] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull } ?: emptyList()

    private inline fun <reified T> JsonObject.decode(key: String): T? {
        val e = this[key] ?: return null
        return runCatching { json.decodeFromJsonElement<T>(e) }.getOrNull()
    }

    private inline fun <reified T> JsonObject.decodeList(key: String): List<T> {
        val arr = this[key] as? JsonArray ?: return emptyList()
        return arr.mapNotNull { e ->
            runCatching { json.decodeFromJsonElement<T>(e) }.getOrNull()
        }
    }
}
