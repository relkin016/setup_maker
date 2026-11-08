//! Total RAM (to pick a safe number of compression threads).

#[cfg(target_os = "linux")]
pub fn total_ram_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = s.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(windows)]
pub fn total_ram_bytes() -> Option<u64> {
    #[repr(C)]
    struct MemStatus {
        len: u32,
        load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page: u64,
        avail_page: u64,
        total_virt: u64,
        avail_virt: u64,
        avail_ext: u64,
    }
    extern "system" {
        fn GlobalMemoryStatusEx(m: *mut MemStatus) -> i32;
    }
    let mut m = MemStatus { len: std::mem::size_of::<MemStatus>() as u32, load: 0, total_phys: 0, avail_phys: 0, total_page: 0, avail_page: 0, total_virt: 0, avail_virt: 0, avail_ext: 0 };
    if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 {
        Some(m.total_phys)
    } else {
        None
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn total_ram_bytes() -> Option<u64> {
    None
}
