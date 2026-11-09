use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stats {
    pub cpu_pct: f64,
    pub mem_used: u64,
    pub mem_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub blk_read: u64,
    pub blk_write: u64,
    pub pids: u64,
}

pub fn parse(raw: &Value) -> Stats {
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let cpu = &raw["cpu_stats"];
    let pre = &raw["precpu_stats"];
    let used = n(&cpu["cpu_usage"]["total_usage"]).saturating_sub(n(&pre["cpu_usage"]["total_usage"]));
    let system = n(&cpu["system_cpu_usage"]).saturating_sub(n(&pre["system_cpu_usage"]));
    let cpus = match n(&cpu["online_cpus"]) {
        0 => cpu["cpu_usage"]["percpu_usage"]
            .as_array()
            .map_or(1, |a| a.len() as u64),
        cpus => cpus,
    };
    let cpu_pct = match system {
        0 => 0.0,
        system => used as f64 / system as f64 * cpus as f64 * 100.0,
    };
    let memory = &raw["memory_stats"];
    let cache = memory["stats"]["inactive_file"]
        .as_u64()
        .or_else(|| memory["stats"]["total_inactive_file"].as_u64())
        .unwrap_or(0);
    let networks = raw["networks"].as_object().into_iter().flat_map(|m| m.values());
    let (net_rx, net_tx) = networks.fold((0, 0), |(rx, tx), net| {
        (rx + n(&net["rx_bytes"]), tx + n(&net["tx_bytes"]))
    });
    let blk = |op: &str| {
        raw["blkio_stats"]["io_service_bytes_recursive"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["op"].as_str().is_some_and(|o| o.eq_ignore_ascii_case(op)))
            .map(|row| n(&row["value"]))
            .sum()
    };
    Stats {
        cpu_pct,
        mem_used: n(&memory["usage"]).saturating_sub(cache),
        mem_limit: n(&memory["limit"]),
        net_rx,
        net_tx,
        blk_read: blk("read"),
        blk_write: blk("write"),
        pids: n(&raw["pids_stats"]["current"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/stats.json")).unwrap()
    }

    #[test]
    fn stats_read_as_docker_stats_shows_them() {
        let stats = parse(&fixture());
        // (138168000 - 114906000) / (355230330000000 - 355214400000000) * 16 * 100
        assert!((stats.cpu_pct - 2.3364).abs() < 0.001, "{stats:?}");
        assert_eq!(stats.mem_used, 1568768 - 221184);
        assert_eq!(stats.mem_limit, 67108864);
        assert_eq!((stats.net_rx, stats.net_tx), (4469, 126));
        assert_eq!((stats.blk_read, stats.blk_write), (299008, 0));
        assert_eq!(stats.pids, 2);
    }

    #[test]
    fn cgroup_v1_names_its_page_cache_total_inactive_file() {
        let mut raw = fixture();
        raw["memory_stats"]["stats"] = json!({ "total_inactive_file": 1000 });
        assert_eq!(parse(&raw).mem_used, 1568768 - 1000);
    }

    #[test]
    fn a_first_sample_with_no_previous_one_is_zero_rather_than_a_division() {
        let mut raw = fixture();
        raw["precpu_stats"] = json!({});
        raw["cpu_stats"]["system_cpu_usage"] = json!(0);
        assert_eq!(parse(&raw).cpu_pct, 0.0);
    }
}
