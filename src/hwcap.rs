use crate::elf::ElfArch;
use crate::error::LdconfigError;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum HwCap {
    Haswell,
    Avx512,
    Sse,
    Sve2,
    Power9,
    Custom(String),
}

impl HwCap {
    pub fn from_path_component(component: &str) -> Option<Self> {
        match component {
            // x86_64 microarchitecture levels
            "x86-64-v2" | "x86-64-v3" | "x86-64-v4" => Some(HwCap::Custom(component.to_string())),
            "haswell" => Some(HwCap::Haswell),
            "avx512" => Some(HwCap::Avx512),
            "sse" => Some(HwCap::Sse),
            // ARM variants
            "asimd" | "neon" => Some(HwCap::Custom(component.to_string())),
            "sve2" => Some(HwCap::Sve2),
            // PowerPC variants
            "power9" | "power10" => Some(HwCap::Power9),
            _ => None,
        }
    }

    /// Convert hwcap to bitmask using kernel-accurate values
    /// These values are architecture-specific and match Linux kernel AT_HWCAP
    pub fn to_bitmask(&self, arch: ElfArch) -> u64 {
        match (arch, self) {
            // x86_64 microarchitecture levels (glibc-hwcaps)
            (ElfArch::X86_64, HwCap::Custom(s)) if s == "x86-64-v2" => 0x01,
            (ElfArch::X86_64, HwCap::Custom(s)) if s == "x86-64-v3" => 0x02,
            (ElfArch::X86_64, HwCap::Custom(s)) if s == "x86-64-v4" => 0x04,
            (ElfArch::X86_64, HwCap::Haswell) => 0x02,  // AVX2 level
            (ElfArch::X86_64, HwCap::Avx512) => 0x04,   // AVX-512 level
            (ElfArch::X86_64, HwCap::Sse) => 0x00,      // Baseline, no special hwcap

            // ARM64 hwcaps (from Linux kernel)
            (ElfArch::AArch64, HwCap::Custom(s)) if s == "asimd" => 1 << 1,
            (ElfArch::AArch64, HwCap::Custom(s)) if s == "neon" => 1 << 1,
            (ElfArch::AArch64, HwCap::Sve2) => 1 << 2,

            // PowerPC hwcaps
            (ElfArch::PowerPC64, HwCap::Power9) => 1 << 0,
            (ElfArch::PowerPC64, HwCap::Custom(s)) if s == "power10" => 1 << 1,

            // Default: no hwcap
            _ => 0,
        }
    }
}

pub fn detect_hwcap_dirs(base_dir: &Path) -> Result<Vec<(PathBuf, HwCap)>, LdconfigError> {
    let mut hwcap_dirs = Vec::new();

    if !base_dir.exists() {
        return Ok(hwcap_dirs);
    }

    for entry in fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(component) = path.file_name() {
                if let Some(hwcap) =
                    HwCap::from_path_component(component.to_string_lossy().as_ref())
                {
                    hwcap_dirs.push((path, hwcap));
                }
            }
        }
    }

    Ok(hwcap_dirs)
}

pub fn scan_hwcap_libraries(
    hwcap_dirs: &[(PathBuf, HwCap)],
) -> Result<Vec<(PathBuf, HwCap)>, LdconfigError> {
    let mut libraries = Vec::new();

    for (dir, hwcap) in hwcap_dirs {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && is_shared_library(&path) {
                libraries.push((path, hwcap.clone()));
            }
        }
    }

    Ok(libraries)
}

fn is_shared_library(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        if ext == "so" {
            return true;
        }
    }

    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.contains(".so."))
        .unwrap_or(false)
}
