//! Host information for Settings ▸ Memory & CPU (RAM reserved for other applications, reducing
//! caches when the system is low on memory) and Help ▸ System Compatibility Report.
//!
//! Pure Rust and dependency-free: the values come from the operating system's own reporting
//! (`/proc/meminfo` on Linux, `sysctl`/`vm_stat` on macOS, `wmic`/PowerShell on Windows). Every
//! query may fail (sandboxes, the web): callers treat `None` as "unknown" and keep their
//! configured budgets.

use serde::Serialize;

/// Physical memory in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SysMemory {
    pub total: u64,
    /// Memory the system can hand out without swapping (free + reclaimable caches).
    pub available: u64,
}

impl SysMemory {
    /// The system is low on memory: less than 10% (or less than 1 GB) is available.
    pub fn is_low(&self) -> bool {
        self.total > 0 && (self.available < self.total / 10 || self.available < 1 << 30)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Parse `/proc/meminfo` (kB values).
pub fn parse_meminfo(text: &str) -> Option<SysMemory> {
    let field = |name: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(name))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:").or_else(|| Some(field("MemFree:")? + field("Cached:").unwrap_or(0)))?;
    Some(SysMemory { total, available })
}

/// Parse macOS `vm_stat` output: free + inactive + speculative + purgeable pages.
pub fn parse_vm_stat(text: &str, total: u64) -> Option<SysMemory> {
    let page: u64 = text.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |name: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.rsplit(':').next())
            .and_then(|v| v.trim().trim_end_matches('.').parse::<u64>().ok())
            .unwrap_or(0)
    };
    let free = pages("Pages free") + pages("Pages inactive") + pages("Pages speculative") + pages("Pages purgeable");
    Some(SysMemory { total, available: (free * page).min(total) })
}

/// Total and available physical memory, when the system reports them.
pub fn memory() -> Option<SysMemory> {
    #[cfg(target_os = "linux")]
    {
        return parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?);
    }
    #[cfg(target_os = "macos")]
    {
        let total: u64 = run("sysctl", &["-n", "hw.memsize"])?.trim().parse().ok()?;
        return parse_vm_stat(&run("vm_stat", &[])?, total).or(Some(SysMemory { total, available: total }));
    }
    #[cfg(target_os = "windows")]
    {
        let text = run(
            "powershell",
            &["-NoProfile", "-Command", "$o=Get-CimInstance Win32_OperatingSystem; \"$($o.TotalVisibleMemorySize) $($o.FreePhysicalMemory)\""],
        )?;
        let mut it = text.split_whitespace().filter_map(|v| v.parse::<u64>().ok());
        return Some(SysMemory { total: it.next()? * 1024, available: it.next()? * 1024 });
    }
    #[allow(unreachable_code)]
    None
}

/// CPU model name.
pub fn cpu_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let t = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        return t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_string());
    }
    #[cfg(target_os = "macos")]
    {
        return run("sysctl", &["-n", "machdep.cpu.brand_string"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var("PROCESSOR_IDENTIFIER").ok();
    }
    #[allow(unreachable_code)]
    None
}

/// Operating system name and version.
pub fn os_version() -> String {
    #[cfg(target_os = "macos")]
    if let Some(v) = run("sw_vers", &["-productVersion"]) {
        return format!("macOS {}", v.trim());
    }
    #[cfg(target_os = "linux")]
    if let Ok(t) = std::fs::read_to_string("/etc/os-release")
        && let Some(n) = t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))
    {
        return n.trim_matches('"').to_string();
    }
    #[cfg(target_os = "windows")]
    if let Some(v) = run("cmd", &["/C", "ver"]) {
        return v.trim().to_string();
    }
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

/// Logical CPU cores.
pub fn cpu_cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_meminfo_and_vm_stat() {
        let m = parse_meminfo("MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:    8000000 kB\n").unwrap();
        assert_eq!(m, SysMemory { total: 16_000_000 * 1024, available: 8_000_000 * 1024 });
        assert!(!m.is_low());
        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               1000.\nPages active:  5.\nPages inactive:                           3000.\nPages speculative:                         100.\n";
        let m = parse_vm_stat(vm, 8 << 30).unwrap();
        assert_eq!(m.available, 4100 * 16384);
        assert!(m.is_low());
    }
}
