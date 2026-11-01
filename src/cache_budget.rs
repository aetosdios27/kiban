//! Default block-cache budget, bounded by host and cgroup v2 memory.

const MIB: u64 = 1 << 20;

fn budget(physical: Option<u64>, cgroup: Option<u64>) -> usize {
    let memory = match (physical, cgroup) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return 32 * MIB as usize,
    };
    (memory / 64).clamp(32 * MIB, 512 * MIB) as usize
}

fn memory_max(raw: &str) -> Option<u64> {
    raw.trim().parse().ok()
}

pub(crate) fn default_block_cache_bytes() -> usize {
    let physical = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|raw| {
            let line = raw.lines().find(|line| line.starts_with("MemTotal:"))?;
            line.split_whitespace()
                .nth(1)?
                .parse::<u64>()
                .ok()?
                .checked_mul(1024)
        });
    let root = std::path::Path::new("/sys/fs/cgroup");
    let mut cgroup = None;
    if let Ok(raw) = std::fs::read_to_string("/proc/self/cgroup")
        && let Some(path) = raw.lines().find_map(|line| line.strip_prefix("0::/"))
    {
        // A parent's finite limit also constrains this process, even when
        // its own memory.max says "max". The root covers cgroup namespaces.
        let current = root.join(path);
        for directory in current.ancestors().take_while(|p| p.starts_with(root)) {
            if let Ok(raw) = std::fs::read_to_string(directory.join("memory.max"))
                && let Some(limit) = memory_max(&raw)
            {
                cgroup = Some(cgroup.map_or(limit, |old: u64| old.min(limit)));
            }
        }
    }
    budget(physical, cgroup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_respect_limits_floor_cap_and_missing_information() {
        let gib = 1024 * MIB;
        for (physical, cgroup, expected) in [
            (None, None, 32 * MIB),
            (Some(0), None, 32 * MIB),
            (Some(gib), None, 32 * MIB),
            (Some(8 * gib), None, 128 * MIB),
            (Some(30 * gib), None, 480 * MIB),
            (Some(64 * gib), None, 512 * MIB),
            (Some(u64::MAX), None, 512 * MIB),
            (Some(64 * gib), Some(4 * gib), 64 * MIB),
            (Some(4 * gib), Some(64 * gib), 64 * MIB),
            (Some(64 * gib), Some(0), 32 * MIB),
            (None, Some(8 * gib), 128 * MIB),
        ] {
            assert_eq!(budget(physical, cgroup), expected as usize);
        }
    }

    #[test]
    fn cgroup_limit_parsing() {
        assert_eq!(memory_max("max\n"), None);
        assert_eq!(memory_max("4294967296\n"), Some(4 << 30));
        assert_eq!(memory_max("0"), Some(0));
        assert_eq!(memory_max("bad"), None);
        assert_eq!(memory_max("-1"), None);
        assert_eq!(memory_max("18446744073709551616"), None);
    }
}
