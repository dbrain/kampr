package dev.kampr.shared.net

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
enum class FsKind {
    @SerialName("dir")
    Dir,

    @SerialName("file")
    File,

    @SerialName("link")
    Link,

    @SerialName("other")
    Other,
}

@Serializable
data class FsEntry(
    val name: String,
    val kind: FsKind = FsKind.Other,
    @SerialName("target_kind") val targetKind: FsKind? = null,
    val bytes: Long? = null,
    val mtime: Long? = null,
    val git: String? = null,
) {
    val isDir: Boolean get() = kind == FsKind.Dir || (kind == FsKind.Link && targetKind == FsKind.Dir)
}

@Serializable
data class RepoHead(
    val root: String,
    val branch: String? = null,
    val head: String? = null,
    val upstream: String? = null,
    val ahead: Int = 0,
    val behind: Int = 0,
    val changed: Int = 0,
)

@Serializable
data class FsListing(
    val path: String,
    val parent: String? = null,
    val home: String? = null,
    val entries: List<FsEntry> = emptyList(),
    val truncated: Boolean = false,
    val repo: RepoHead? = null,
)

@Serializable
data class RepoChange(
    val path: String,
    val abs: String,
    val staged: String = " ",
    val unstaged: String = " ",
    val orig: String? = null,
)

@Serializable
data class RepoCommit(
    val sha: String,
    val short: String,
    val subject: String = "",
    val author: String = "",
    val `when`: Long = 0,
)

@Serializable
data class RepoSummary(
    val root: String,
    val branch: String? = null,
    val head: String? = null,
    val upstream: String? = null,
    val ahead: Int = 0,
    val behind: Int = 0,
    val changes: List<RepoChange> = emptyList(),
    val log: List<RepoCommit> = emptyList(),
)

// A kind a newer node invents reads as `other` rather than failing the whole listing.
private val json = Json {
    ignoreUnknownKeys = true
    coerceInputValues = true
    explicitNulls = false
}

fun decodeListing(bytes: ByteArray): FsListing? = runCatching { json.decodeFromString<FsListing>(bytes.decodeToString()) }.getOrNull()

fun decodeRepo(bytes: ByteArray): RepoSummary? = runCatching { json.decodeFromString<RepoSummary>(bytes.decodeToString()) }.getOrNull()

// Path forms are routed by node id alone, so a node with no pane on screen is still browsable.
fun anchorPane(nodeId: String): String = "$nodeId/_"
