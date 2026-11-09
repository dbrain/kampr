//! ELF headers that say what a test needs said: a machine, and which program headers it has.

pub const AMD64: u16 = 0x3e;
pub const ARM64: u16 = 0xb7;
pub const PT_INTERP: u32 = 3;

/// A 64-bit little-endian ELF header followed by one program header per entry of `kinds`.
pub fn elf64(machine: u16, kinds: &[u32]) -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&u16::try_from(kinds.len()).unwrap().to_le_bytes());
    for kind in kinds {
        let mut header = vec![0u8; 56];
        header[..4].copy_from_slice(&kind.to_le_bytes());
        bytes.extend(header);
    }
    bytes
}

/// One that runs without a loader on `machine`.
pub fn static_elf(machine: u16) -> Vec<u8> {
    elf64(machine, &[6, 1, 1, 2])
}

/// One that names a loader, as a glibc build does.
pub fn dynamic_elf(machine: u16) -> Vec<u8> {
    elf64(machine, &[6, PT_INTERP, 1, 1, 2])
}
