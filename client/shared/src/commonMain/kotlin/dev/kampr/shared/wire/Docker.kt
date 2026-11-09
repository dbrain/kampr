package dev.kampr.shared.wire

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

// `decodeList` drops a whole node when any of it fails to decode, so one odd container from a newer
// node would take its host off the herd. Each entry is decoded on its own and a bad one is skipped.
open class LenientList<T>(private val element: KSerializer<T>) : KSerializer<List<T>> {
    private val strict = ListSerializer(element)
    override val descriptor = strict.descriptor

    override fun deserialize(decoder: Decoder): List<T> {
        val json = decoder as? JsonDecoder ?: return strict.deserialize(decoder)
        val array = json.decodeJsonElement() as? JsonArray ?: return emptyList()
        return array.mapNotNull { runCatching { json.json.decodeFromJsonElement(element, it) }.getOrNull() }
    }

    override fun serialize(encoder: Encoder, value: List<T>) = strict.serialize(encoder, value)
}

object ContainerList : LenientList<ContainerInfo>(ContainerInfo.serializer())

object StringList : LenientList<String>(String.serializer())

object MountList : LenientList<DockerMount>(DockerMount.serializer())

object ProbeList : LenientList<HealthProbe>(HealthProbe.serializer())

object LogList : LenientList<LogLine>(LogLine.serializer())

object EventList : LenientList<DockerEvent>(DockerEvent.serializer())

@Serializable
data class CrashInfo(
    val at: Long = 0,
    @SerialName("exit_code") val exitCode: Int = 0,
    val oom: Boolean = false,
)

@Serializable
data class ContainerInfo(
    val id: String = "",
    val name: String = "",
    val image: String = "",
    val state: String = "",
    val health: String? = null,
    @SerialName("started_at") val startedAt: Long? = null,
    @SerialName("finished_at") val finishedAt: Long? = null,
    @SerialName("exit_code") val exitCode: Int? = null,
    val oom: Boolean = false,
    val restarts: Int = 0,
    val crashes: Int = 0,
    @SerialName("last_crash") val lastCrash: CrashInfo? = null,
    val project: String? = null,
    val service: String? = null,
    @Serializable(with = StringList::class) val ports: List<String> = emptyList(),
    val child: String? = null,
    @SerialName("adopt_error") val adoptError: String? = null,
)

@Serializable
data class DockerMount(val src: String = "", val dst: String = "", val mode: String = "")

@Serializable
data class HealthProbe(
    val start: String? = null,
    val end: String? = null,
    @SerialName("exit_code") val exitCode: Int? = null,
    val output: String = "",
)

@Serializable
data class LogLine(val stream: String? = null, val line: String = "")

@Serializable
data class DockerEvent(
    val at: Long = 0,
    val action: String = "",
    @SerialName("exit_code") val exitCode: Int? = null,
    val signal: String? = null,
    val health: String? = null,
)

@Serializable
data class DockerStats(
    @SerialName("cpu_pct") val cpuPct: Double? = null,
    @SerialName("mem_used") val memUsed: Long? = null,
    @SerialName("mem_limit") val memLimit: Long? = null,
    @SerialName("net_rx") val netRx: Long? = null,
    @SerialName("net_tx") val netTx: Long? = null,
    @SerialName("blk_read") val blkRead: Long? = null,
    @SerialName("blk_write") val blkWrite: Long? = null,
    val pids: Int? = null,
)

@Serializable
data class DockerLimits(val memory: Long? = null, @SerialName("nano_cpus") val nanoCpus: Long? = null)

// The half of `docker.detail` the herd entry does not carry. `command`, `created` and
// `restart_policy` are shown and never read, so whatever shape the node gives them is drawn flat.
@Serializable
data class ContainerMore(
    val command: JsonElement? = null,
    val created: JsonElement? = null,
    @SerialName("restart_policy") val restartPolicy: JsonElement? = null,
    val limits: DockerLimits? = null,
    @Serializable(with = StringList::class) val env: List<String> = emptyList(),
    val labels: Map<String, String> = emptyMap(),
    @Serializable(with = MountList::class) val mounts: List<DockerMount> = emptyList(),
    @Serializable(with = StringList::class) val networks: List<String> = emptyList(),
    @SerialName("health_log") @Serializable(with = ProbeList::class) val healthLog: List<HealthProbe> = emptyList(),
    @Serializable(with = LogList::class) val logs: List<LogLine> = emptyList(),
    val stats: DockerStats? = null,
    @Serializable(with = EventList::class) val events: List<DockerEvent> = emptyList(),
)

data class DockerDetail(val entry: ContainerInfo, val more: ContainerMore)

fun dockerDetailOf(obj: JsonObject?): DockerDetail? {
    obj ?: return null
    val entry = runCatching { Wire.json.decodeFromJsonElement(ContainerInfo.serializer(), obj) }.getOrNull() ?: return null
    val more = runCatching { Wire.json.decodeFromJsonElement(ContainerMore.serializer(), obj) }.getOrNull() ?: ContainerMore()
    return DockerDetail(entry, more)
}

fun flat(element: JsonElement?): String? = when (element) {
    null -> null
    is JsonPrimitive -> element.contentOrNull
    is JsonArray -> element.mapNotNull { flat(it) }.joinToString(" ")
    is JsonObject -> element.entries.joinToString(", ") { (k, v) -> "$k ${flat(v).orEmpty()}" }
}?.takeIf { it.isNotBlank() }
