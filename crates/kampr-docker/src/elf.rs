use anyhow::{Result, bail};
use std::io::Read;
use std::path::Path;

const PT_INTERP: u32 = 3;
const HEAD: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf {
    /// Docker's name for it (`amd64`, `arm64`), so it compares with an image's `Architecture`.
    pub arch: String,
    /// Names a dynamic loader, which a container's libc may not have, or have at another version.
    pub interpreted: bool,
}

pub fn read(path: &Path) -> Result<Elf> {
    let mut head = Vec::with_capacity(HEAD);
    std::fs::File::open(path)?
        .take(HEAD as u64)
        .read_to_end(&mut head)?;
    parse(&head)
}

pub fn parse(bytes: &[u8]) -> Result<Elf> {
    if bytes.len() < 52 || &bytes[..4] != b"\x7fELF" {
        bail!("not an ELF executable");
    }
    let wide = match bytes[4] {
        1 => false,
        2 => true,
        _ => bail!("an ELF of no class"),
    };
    let little = match bytes[5] {
        1 => true,
        2 => false,
        _ => bail!("an ELF of no byte order"),
    };
    let u16_at = |at: usize| -> Option<u64> {
        let b: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
        Some(u64::from(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }))
    };
    let u32_at = |at: usize| -> Option<u64> {
        let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(u64::from(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }))
    };
    let u64_at = |at: usize| -> Option<u64> {
        let b: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
        Some(if little {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        })
    };
    let machine = u16_at(18).unwrap_or(0);
    let (phoff, entsize, count) = match wide {
        true => (u64_at(32), u16_at(54), u16_at(56)),
        false => (u32_at(28), u16_at(42), u16_at(44)),
    };
    let (Some(phoff), Some(entsize), Some(count)) = (phoff, entsize, count) else {
        bail!("a truncated ELF header");
    };
    let mut interpreted = false;
    for n in 0..count {
        let at = usize::try_from(phoff + n * entsize)?;
        let Some(kind) = u32_at(at) else {
            bail!("program headers past the first {} bytes", bytes.len());
        };
        interpreted |= kind == u64::from(PT_INTERP);
    }
    Ok(Elf {
        arch: arch_of(machine).to_string(),
        interpreted,
    })
}

fn arch_of(machine: u64) -> String {
    match machine {
        0x3e => "amd64".into(),
        0xb7 => "arm64".into(),
        0x28 => "arm".into(),
        0x03 => "386".into(),
        0xf3 => "riscv64".into(),
        0x15 => "ppc64le".into(),
        0x16 => "s390x".into(),
        other => format!("elf-machine-{other:#x}"),
    }
}

/// `uname -m` and Docker spell the same machines differently.
pub fn same_arch(a: &str, b: &str) -> bool {
    normal(a) == normal(b)
}

/// The published static release that runs on `arch`, in either spelling.
pub fn musl_target(arch: &str) -> Option<&'static str> {
    match normal(arch) {
        "amd64" => Some("x86_64-unknown-linux-musl"),
        "arm64" => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

fn normal(arch: &str) -> &str {
    match arch.trim() {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        "i386" | "i686" | "386" => "386",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_testkit::elf::elf64;

    #[test]
    fn a_binary_that_names_a_loader_is_not_one_a_container_can_run_as_copied() {
        for (kinds, interpreted) in [
            (vec![6, 3, 1, 1, 2], true),
            (vec![6, 1, 1, 2, 7], false),
            (vec![], false),
        ] {
            let elf = parse(&elf64(0x3e, &kinds)).unwrap();
            assert_eq!(elf.interpreted, interpreted, "{kinds:?}");
            assert_eq!(elf.arch, "amd64");
        }
        assert_eq!(parse(&elf64(0xb7, &[1])).unwrap().arch, "arm64");
        assert!(
            parse(b"#!/bin/sh\necho not elf\n.................................................").is_err()
        );
        let mut cut = elf64(0x3e, &[1, 3]);
        cut.truncate(64 + 56);
        assert!(parse(&cut).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_test_binary_itself_reads_as_this_machines_architecture() {
        let elf = read(&std::env::current_exe().unwrap()).unwrap();
        assert!(same_arch(&elf.arch, std::env::consts::ARCH), "{elf:?}");
    }

    #[test]
    fn an_architecture_maps_to_the_release_built_for_it() {
        for (arch, target) in [
            ("amd64", Some("x86_64-unknown-linux-musl")),
            ("x86_64", Some("x86_64-unknown-linux-musl")),
            ("arm64", Some("aarch64-unknown-linux-musl")),
            ("aarch64", Some("aarch64-unknown-linux-musl")),
            ("arm", None),
            ("riscv64", None),
            ("", None),
        ] {
            assert_eq!(musl_target(arch), target, "{arch}");
        }
    }

    #[test]
    fn architectures_compare_across_both_spellings() {
        for (a, b, same) in [
            ("amd64", "x86_64", true),
            ("x86_64", "amd64", true),
            ("arm64", "aarch64", true),
            ("aarch64", "arm64", true),
            ("amd64", "amd64", true),
            ("amd64", "arm64", false),
            ("x86_64", "aarch64", false),
            ("arm", "arm64", false),
        ] {
            assert_eq!(same_arch(a, b), same, "{a} {b}");
        }
    }
}
