//! A static `kampr` built from this tree, for a suite that copies it into a container. A glibc
//! build names the host's loader and libc, which an Alpine image does not have and a Debian one
//! has at an older version (docs/19).
//!
//! Rust's musl target links its own libc, but `ring`, `sqlite` and the vendored OpenSSL compile C,
//! which needs musl's headers. A machine without `musl-gcc` gets them from Alpine's `musl-dev`
//! package, unpacked once into a sysroot that `clang --target` builds against.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const ALPINE: &str = "https://dl-cdn.alpinelinux.org/alpine/latest-stable/main";

/// The binary, or why there is none, in words that say what to install.
pub fn static_kampr() -> Result<PathBuf, String> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    BUILT.get_or_init(build).clone()
}

fn build() -> Result<PathBuf, String> {
    if let Some(given) = std::env::var_os("KAMPR_STATIC_BIN") {
        return Ok(PathBuf::from(given));
    }
    let arch = std::env::consts::ARCH;
    let target = format!("{arch}-unknown-linux-musl");
    rust_target(&target)?;
    crate::on_path("clang").ok_or("no clang to compile the C dependencies for musl with")?;
    let sysroot = sysroot(arch)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into());
    let env_target = target.replace('-', "_");
    let mut build = Command::new(cargo);
    build
        .args(["build", "--quiet", "-p", "kampr-cli", "--bin", "kampr"])
        .args(["--features", "vendored-tls", "--target", &target])
        .env(format!("CC_{env_target}"), "clang")
        .env(
            format!("CFLAGS_{env_target}"),
            format!("--target={arch}-linux-musl --sysroot={}", sysroot.display()),
        );
    if let Some(ar) = crate::on_path("llvm-ar") {
        build.env(format!("AR_{env_target}"), ar);
    }
    let status = build
        .status()
        .map_err(|e| format!("cargo build --target {target}: {e}"))?;
    if !status.success() {
        return Err(format!("cargo build --target {target} failed"));
    }
    let binary = crate::keeper::target_dir()
        .join(&target)
        .join("debug")
        .join("kampr");
    match binary.is_file() {
        true => Ok(binary),
        false => Err(format!("no kampr at {}", binary.display())),
    }
}

fn rust_target(target: &str) -> Result<(), String> {
    let out = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", target])
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    let libdir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    match out.status.success() && libdir.is_dir() {
        true => Ok(()),
        false => Err(format!(
            "no {target} standard library: rustup target add {target}"
        )),
    }
}

fn sysroot(arch: &str) -> Result<PathBuf, String> {
    let root = match std::env::var_os("KAMPR_MUSL_SYSROOT") {
        Some(root) => PathBuf::from(root),
        None => {
            let cache = std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))
                .ok_or("no cache directory for a musl sysroot")?;
            cache.join("kampr").join(format!("musl-sysroot-{arch}"))
        }
    };
    if root.join("usr/include/stdio.h").is_file() {
        return Ok(root);
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let index = format!("{ALPINE}/{arch}/APKINDEX.tar.gz");
    let script = format!(
        "set -e; v=$(curl -sfL {index} | tar xzO APKINDEX | awk '/^P:musl-dev$/{{p=1}} p&&/^V:/{{print substr($0,3); exit}}'); \
         test -n \"$v\"; curl -sfL {ALPINE}/{arch}/musl-dev-$v.apk | tar xz -C . 2>/dev/null; test -f usr/include/stdio.h"
    );
    let fetched = Command::new("sh")
        .args(["-c", &script])
        .current_dir(&root)
        .status()
        .is_ok_and(|s| s.success());
    match fetched {
        true => Ok(root),
        false => Err(format!(
            "no musl headers at {} and none fetched from {ALPINE}; set KAMPR_MUSL_SYSROOT or KAMPR_STATIC_BIN",
            root.display()
        )),
    }
}
