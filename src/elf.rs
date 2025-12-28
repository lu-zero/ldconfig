use camino::Utf8PathBuf;
use goblin::elf::header::ET_DYN;
use goblin::elf::Elf;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Not a shared object (ET_DYN)")]
    NotSharedObject,

    #[error("Missing PT_DYNAMIC segment")]
    MissingDynamicSegment,

    #[error("Missing DT_SONAME entry")]
    MissingSoname,

    #[error("Empty SONAME")]
    EmptySoname,

    #[error("Unsupported ELF class")]
    UnsupportedClass,

    #[error("Unsupported endianness")]
    UnsupportedEndianness,

    #[error("Unsupported architecture")]
    UnsupportedArchitecture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfArch {
    X86_64,
    AArch64,
    RiscV64,
    PowerPC64,
    IA64,
    I686,
    ARM,
}

#[derive(Debug, Clone)]
pub struct ElfLibrary {
    pub soname: String,
    pub path: Utf8PathBuf,
    pub is_64bit: bool,
    pub arch: ElfArch,
    pub is_hardfloat: bool,
    pub osversion: u32,
    pub hwcap: Option<u64>,
}

pub fn parse_elf_file(path: &Path) -> Result<ElfLibrary, crate::Error> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    let elf = Elf::parse(&mmap)?;

    validate_elf(&elf)?;
    let soname = extract_soname(&elf, path)?;
    let arch = detect_architecture(&elf)?;
    let is_hardfloat = detect_hardfloat(&elf);

    // Convert Path to Utf8PathBuf
    let utf8_path = Utf8PathBuf::try_from(path.to_path_buf())
        .map_err(|_| crate::Error::Config("Path contains invalid UTF-8".to_string()))?;

    Ok(ElfLibrary {
        soname,
        path: utf8_path,
        is_64bit: elf.is_64,
        arch,
        is_hardfloat,
        osversion: extract_osversion(&elf),
        hwcap: detect_hwcap_from_path(path),
    })
}

fn validate_elf(elf: &Elf) -> Result<(), Error> {
    // Must be a shared object (ET_DYN)
    if elf.header.e_type != ET_DYN {
        return Err(Error::NotSharedObject);
    }

    // Must have PT_DYNAMIC segment
    if elf
        .program_headers
        .iter()
        .all(|ph| ph.p_type != goblin::elf::program_header::PT_DYNAMIC)
    {
        return Err(Error::MissingDynamicSegment);
    }

    Ok(())
}

fn extract_soname(elf: &Elf, _path: &Path) -> Result<String, crate::Error> {
    let soname_index = match &elf.dynamic {
        Some(dynamic) => dynamic.info.soname,
        None => return Err(Error::MissingSoname.into()),
    };

    let soname_str = match elf.dynstrtab.get_at(soname_index) {
        Some(s) => s,
        None => return Err(Error::MissingSoname.into()),
    };

    if soname_str.is_empty() {
        return Err(Error::EmptySoname.into());
    }

    Ok(soname_str.to_string())
}

fn detect_architecture(elf: &Elf) -> Result<ElfArch, Error> {
    use goblin::elf::header::*;
    match elf.header.e_machine {
        EM_X86_64 => Ok(ElfArch::X86_64),
        EM_AARCH64 => Ok(ElfArch::AArch64),
        EM_RISCV => Ok(ElfArch::RiscV64),
        EM_PPC64 => Ok(ElfArch::PowerPC64),
        EM_IA_64 => Ok(ElfArch::IA64),
        EM_386 => Ok(ElfArch::I686),
        EM_ARM => Ok(ElfArch::ARM),
        _ => {
            // Use goblin's machine_to_str for better error messages
            let machine_str = machine_to_str(elf.header.e_machine);
            eprintln!("Unsupported architecture: {} (0x{:x})", machine_str, elf.header.e_machine);
            Err(Error::UnsupportedArchitecture)
        }
    }
}

fn detect_hardfloat(elf: &Elf) -> bool {
    // Check ELF flags for hard-float ABI (EF_ARM_ABI_FLOAT_HARD)
    if elf.header.e_machine == goblin::elf::header::EM_ARM {
        (elf.header.e_flags & 0x400) != 0
    } else {
        false
    }
}

fn extract_osversion(_elf: &Elf) -> u32 {
    // Search for PT_NOTE segment with NT_GNU_ABI_TAG
    // Note format: namesz (4), descsz (4), type (4), name, desc
    // ABI tag desc: OS (4), major (4), minor (4), patch (4)
    // Returns: (major << 24) | (minor << 16) | patch

    // For now, return 0 (no version requirement)
    // Full implementation requires parsing note section binary data
    // from program header PT_NOTE segments
    0
}

fn detect_hwcap_from_path(path: &Path) -> Option<u64> {
    path.components().find_map(|c| {
        let component = c.as_os_str().to_string_lossy();
        match component.as_ref() {
            "haswell" => Some(1 << 0),
            "avx512" => Some(1 << 1),
            "sve2" => Some(1 << 2),
            _ => None,
        }
    })
}
