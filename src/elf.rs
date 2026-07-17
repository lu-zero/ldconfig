//! ELF inspection mirroring glibc's readelflib.c.
//!
//! Like glibc, only the ELF header and program headers are examined;
//! section headers may be stripped or damaged without affecting the scan.

use goblin::container::{Container, Ctx};
use goblin::elf::dynamic::{Dynamic, DT_SONAME};
use goblin::elf::header::{
    Header, EI_DATA, ELFDATA2LSB, ELFDATA2MSB, EM_386, EM_AARCH64, EM_ARM, EM_PPC, EM_PPC64,
    EM_RISCV, EM_X86_64, ET_DYN,
};
use goblin::elf::program_header::{ProgramHeader, PT_DYNAMIC, PT_LOAD};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use tracing::debug;

use crate::cache_format::{
    FLAG_AARCH64_LIB64, FLAG_ARM_LIBHF, FLAG_ARM_LIBSF, FLAG_ELF_LIBC6, FLAG_POWERPC_LIB64,
    FLAG_RISCV_FLOAT_ABI_DOUBLE, FLAG_RISCV_FLOAT_ABI_SOFT, FLAG_X8664_LIB64, FLAG_X8664_LIBX32,
};

const PT_GNU_PROPERTY: u32 = 0x6474_e553;
const NT_GNU_PROPERTY_TYPE_0: u32 = 5;
const GNU_PROPERTY_X86_ISA_1_NEEDED: u32 = 0xc000_8002;

const EF_ARM_EABIMASK: u32 = 0xff00_0000;
const EF_ARM_EABI_VER5: u32 = 0x0500_0000;
const EF_ARM_ABI_FLOAT_SOFT: u32 = 0x200;
const EF_ARM_ABI_FLOAT_HARD: u32 = 0x400;

const EF_RISCV_RVC: u32 = 0x0001;
const EF_RISCV_FLOAT_ABI: u32 = 0x0006;
const EF_RISCV_FLOAT_ABI_SOFT: u32 = 0x0000;
const EF_RISCV_FLOAT_ABI_DOUBLE: u32 = 0x0004;

#[derive(Debug, Clone)]
pub(crate) struct ElfInfo {
    /// DT_SONAME if present; callers fall back to the file name,
    /// like glibc's implicit_soname.
    pub soname: Option<String>,
    pub flags: u32,
    /// x86 ISA level from GNU_PROPERTY_X86_ISA_1_NEEDED, 0 if unmarked.
    pub isa_level: u32,
}

/// Inspect a shared object like glibc's process_elf_file.
/// Returns None for anything that must not be cached.
pub(crate) fn inspect(path: &Path) -> Option<ElfInfo> {
    let file = File::open(path).ok()?;
    // Safety: read-only shared mapping; a concurrent truncation can raise
    // SIGBUS, the same exposure glibc's ldconfig has when mmapping.
    let map = unsafe { Mmap::map(&file).ok()? };
    inspect_bytes(&map, path)
}

fn inspect_bytes(data: &[u8], path: &Path) -> Option<ElfInfo> {
    let header = goblin::elf::Elf::parse_header(data).ok()?;

    let native = if cfg!(target_endian = "little") {
        ELFDATA2LSB
    } else {
        ELFDATA2MSB
    };
    if header.e_ident[EI_DATA] != native {
        debug!("{}: foreign byte order", path.display());
        return None;
    }
    if header.e_type != ET_DYN {
        debug!("{}: not a shared object", path.display());
        return None;
    }

    let is_64 = header.container().ok()? == Container::Big;
    let Some(flags) = machine_flags(&header, is_64) else {
        debug!(
            "{}: unsupported machine/ABI (e_machine {}, e_flags {:#x})",
            path.display(),
            header.e_machine,
            header.e_flags
        );
        return None;
    };

    let ctx = Ctx::new(header.container().ok()?, header.endianness().ok()?);
    let phdrs =
        ProgramHeader::parse(data, header.e_phoff as usize, header.e_phnum as usize, ctx).ok()?;
    if !phdrs.iter().any(|ph| ph.p_type == PT_DYNAMIC) {
        debug!("{}: missing PT_DYNAMIC", path.display());
        return None;
    }

    let soname = read_soname(data, &phdrs, ctx);
    let isa_level = if matches!(header.e_machine, EM_386 | EM_X86_64) {
        read_isa_level(data, &phdrs, is_64)
    } else {
        0
    };

    Some(ElfInfo {
        soname,
        flags,
        isa_level,
    })
}

/// Per-machine cache flags, following the sysdeps readelflib.c variants.
fn machine_flags(h: &Header, is_64: bool) -> Option<u32> {
    match (h.e_machine, is_64) {
        (EM_X86_64, true) => Some(FLAG_X8664_LIB64 | FLAG_ELF_LIBC6),
        (EM_X86_64, false) => Some(FLAG_X8664_LIBX32 | FLAG_ELF_LIBC6),
        // Every ix86 object (i386 through i686) carries EM_386.
        (EM_386, false) => Some(FLAG_ELF_LIBC6),
        (EM_AARCH64, true) => Some(FLAG_AARCH64_LIB64 | FLAG_ELF_LIBC6),
        (EM_ARM, false) => {
            if h.e_flags & EF_ARM_EABIMASK == EF_ARM_EABI_VER5 {
                if h.e_flags & EF_ARM_ABI_FLOAT_HARD != 0 {
                    Some(FLAG_ARM_LIBHF | FLAG_ELF_LIBC6)
                } else if h.e_flags & EF_ARM_ABI_FLOAT_SOFT != 0 {
                    Some(FLAG_ARM_LIBSF | FLAG_ELF_LIBC6)
                } else {
                    // Unmarked objects are compatible with all ABI variants.
                    Some(FLAG_ELF_LIBC6)
                }
            } else {
                Some(FLAG_ELF_LIBC6)
            }
        }
        (EM_PPC64, true) => Some(FLAG_POWERPC_LIB64 | FLAG_ELF_LIBC6),
        (EM_PPC, false) => Some(FLAG_ELF_LIBC6),
        (EM_RISCV, _) => {
            // glibc rejects anything beyond the float ABI and RVC bits.
            if h.e_flags & !(EF_RISCV_FLOAT_ABI | EF_RISCV_RVC) != 0 {
                return None;
            }
            match h.e_flags & EF_RISCV_FLOAT_ABI {
                EF_RISCV_FLOAT_ABI_SOFT => Some(FLAG_RISCV_FLOAT_ABI_SOFT | FLAG_ELF_LIBC6),
                EF_RISCV_FLOAT_ABI_DOUBLE => Some(FLAG_RISCV_FLOAT_ABI_DOUBLE | FLAG_ELF_LIBC6),
                _ => None,
            }
        }
        _ => None,
    }
}

fn read_soname(data: &[u8], phdrs: &[ProgramHeader], ctx: Ctx) -> Option<String> {
    let dynamic = Dynamic::parse(data, phdrs, ctx).ok()??;
    // First DT_SONAME wins, as in glibc.
    let idx = dynamic.dyns.iter().find(|d| d.d_tag == DT_SONAME)?.d_val as usize;

    let off = vaddr_to_offset(phdrs, dynamic.info.strtab as u64)? as usize;
    let end = off.checked_add(dynamic.info.strsz)?.min(data.len());
    let table = data.get(off..end)?;
    let bytes = table.get(idx..)?;
    let nul = bytes.iter().position(|&b| b == 0)?;
    let soname = std::str::from_utf8(&bytes[..nul]).ok()?;
    (!soname.is_empty()).then(|| soname.to_owned())
}

fn vaddr_to_offset(phdrs: &[ProgramHeader], vaddr: u64) -> Option<u64> {
    phdrs
        .iter()
        .find(|ph| ph.p_type == PT_LOAD && vaddr >= ph.p_vaddr && vaddr - ph.p_vaddr < ph.p_filesz)
        .map(|ph| vaddr - ph.p_vaddr + ph.p_offset)
}

/// x86 ISA level from the NT_GNU_PROPERTY_TYPE_0 note
/// (GNU_PROPERTY_X86_ISA_1_NEEDED), following elf/readelflib.c and
/// sysdeps/unix/sysv/linux/x86/elf-read-prop.h.
fn read_isa_level(data: &[u8], phdrs: &[ProgramHeader], is_64: bool) -> u32 {
    let align = if is_64 { 8usize } else { 4 };
    let u32_at = |seg: &[u8], pos: usize| u32::from_ne_bytes(seg[pos..pos + 4].try_into().unwrap());
    let align_up = |v: usize, a: usize| v.div_ceil(a) * a;

    for ph in phdrs {
        if ph.p_type != PT_GNU_PROPERTY || ph.p_align as usize != align {
            continue;
        }
        let Some(seg) = (ph.p_offset as usize)
            .checked_add(ph.p_filesz as usize)
            .and_then(|end| data.get(ph.p_offset as usize..end))
        else {
            continue;
        };

        let mut pos = 0usize;
        while pos + 12 <= seg.len() {
            let namesz = u32_at(seg, pos) as usize;
            let descsz = u32_at(seg, pos + 4) as usize;
            let n_type = u32_at(seg, pos + 8);
            let name_off = pos + 12;
            let desc_off = name_off.saturating_add(align_up(namesz, 4));

            if n_type == NT_GNU_PROPERTY_TYPE_0
                && namesz == 4
                && seg.get(name_off..name_off + 4) == Some(b"GNU\0")
            {
                if descsz < 8 || !descsz.is_multiple_of(align) {
                    return 0;
                }
                let Some(desc_end) = desc_off.checked_add(descsz).filter(|&e| e <= seg.len())
                else {
                    return 0;
                };

                let mut p = desc_off;
                let mut last_type = 0u32;
                while desc_end - p >= 8 {
                    let p_type = u32_at(seg, p);
                    let datasz = u32_at(seg, p + 4) as usize;
                    // Property types must be ascending; ours is the largest
                    // interesting one, so anything past it ends the search.
                    if p_type < last_type || p_type > GNU_PROPERTY_X86_ISA_1_NEEDED {
                        return 0;
                    }
                    let dstart = p + 8;
                    if dstart + datasz > desc_end {
                        return 0;
                    }
                    if p_type == GNU_PROPERTY_X86_ISA_1_NEEDED {
                        if datasz == 4 {
                            let needed = u32_at(seg, dstart);
                            if needed != 0 {
                                return 31 - needed.leading_zeros();
                            }
                        }
                        return 0;
                    }
                    last_type = p_type;
                    p = dstart + align_up(datasz, align);
                }
                return 0;
            }
            pos = desc_off.saturating_add(align_up(descsz, align));
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_format::{FLAG_ELF_LIBC6, FLAG_X8664_LIB64};
    use std::path::Path;

    #[test]
    fn inspect_system_libz() {
        // Only meaningful on an x86-64 host with the usual layout.
        if !cfg!(target_arch = "x86_64") {
            return;
        }
        let path = Path::new("/usr/lib/libz.so.1");
        if !path.exists() {
            return;
        }
        let info = inspect(path).unwrap();
        assert_eq!(info.soname.as_deref(), Some("libz.so.1"));
        assert_eq!(info.flags, FLAG_X8664_LIB64 | FLAG_ELF_LIBC6);
    }

    #[test]
    fn inspect_rejects_non_elf() {
        assert!(inspect(Path::new("/etc/ld.so.conf")).is_none());
    }
}
