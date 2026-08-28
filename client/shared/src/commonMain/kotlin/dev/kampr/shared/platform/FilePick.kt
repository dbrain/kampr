package dev.kampr.shared.platform

// What the operator chose to hand the agent. `name` is a hint at the stem and nothing more — the
// node owns the directory it writes to and derives the extension from the bytes, because an
// extension the sender chose is an extension the sender chose.
class PickedFile(val name: String?, val mime: String?, val bytes: ByteArray)

// Absent rather than present-and-failing: a platform with no picker draws no attach button.
expect val filePickAvailable: Boolean

// Null is "nothing was chosen", which is what backing out of a system picker means and is not a
// failure to report.
expect suspend fun pickFile(): PickedFile?
