//! The release installer as it shipped in this binary, and the one way to run it.
//!
//! **Embedded rather than fetched.** `install.sh` is the thing that decides whether a download is
//! genuine, and fetching the verifier over the same channel as the artefact means whoever can
//! serve a binary can serve a verifier that accepts it. This copy came out of the release the
//! operator already verified when they installed it, and it cannot drift from the one in the tree
//! because it *is* the one in the tree. `kampr update` and a node fetching a build for a container
//! both run it, so there is one verifier.

use std::path::Path;

pub const SCRIPT: &str = include_str!("../../../packaging/install.sh");

/// `sh <script>` for `repo`, with every variable that would loosen verification cleared.
pub fn command(script: &Path, repo: &str) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command
        .arg(script)
        .env("KAMPR_REPO", repo)
        // The escape hatch exists for an operator installing a binary they built themselves. It
        // must not be reachable by an environment variable that happened to be set in the shell
        // that ran this, because what it switches off is the only thing standing between a
        // tampered download and a process that can type into every terminal on this host.
        .env_remove("KAMPR_ALLOW_UNVERIFIED")
        // The same rule, and the larger hole of the two: the base supplies the tarball *and* the
        // SHA256SUMS it is checked against, so a base set in the caller's environment chooses what
        // kampr becomes and gets "checksum verified: yes" printed underneath it. An operator
        // installing from their own base runs `install.sh` and sets it there, deliberately.
        .env_remove("KAMPR_BASE_URL")
        .env_remove("KAMPR_TARGET");
    command
}
