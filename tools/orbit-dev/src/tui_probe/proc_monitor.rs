//! Per-process Linux sampling. CPU and faults are absolute counters since birth.
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, time::Instant};
fn counters(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.into(), v.split_whitespace().next()?.parse().ok()?))
        })
        .collect()
}
#[derive(Default)]
struct Sample {
    values: BTreeMap<String, u64>,
    user: f64,
    system: f64,
}
fn sample(pid: u32, hz: f64) -> Option<Sample> {
    let root = format!("/proc/{pid}");
    let mut v = counters(&fs::read_to_string(format!("{root}/status")).ok()?);
    for name in ["io", "smaps_rollup"] {
        if let Ok(s) = fs::read_to_string(format!("{root}/{name}")) {
            v.extend(counters(&s));
        }
    }
    if let Ok(entries) = fs::read_dir(format!("{root}/fd")) {
        v.insert("fds".into(), entries.filter_map(Result::ok).count() as u64);
    }
    let stat = fs::read_to_string(format!("{root}/stat")).ok()?;
    let f: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    let field = |n: usize| f.get(n)?.parse::<u64>().ok();
    v.insert("minflt".into(), field(7)?);
    v.insert("majflt".into(), field(9)?);
    Some(Sample {
        values: v,
        user: field(11)? as f64 / hz,
        system: field(12)? as f64 / hz,
    })
}
pub(super) struct Monitor {
    pid: u32,
    start: Instant,
    last_at: Instant,
    last: Sample,
    peak: BTreeMap<String, u64>,
    n: u64,
    hz: f64,
}
impl Monitor {
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            start: Instant::now(),
            last_at: Instant::now(),
            last: Sample::default(),
            peak: BTreeMap::new(),
            n: 0,
            hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64,
        }
    }
    pub fn poll(&mut self) -> bool {
        let Some(s) = sample(self.pid, self.hz) else {
            return false;
        };
        for (key, value) in &s.values {
            self.peak
                .entry(key.clone())
                .and_modify(|v| *v = (*v).max(*value))
                .or_insert(*value);
        }
        self.last = s;
        self.n += 1;
        self.last_at = Instant::now();
        true
    }
    pub fn report(&self) -> Value {
        let p = |key: &str| self.peak.get(key).copied().unwrap_or(0);
        let l = |key: &str| self.last.values.get(key).copied().unwrap_or(0);
        let wall = self
            .last_at
            .saturating_duration_since(self.start)
            .as_secs_f64()
            * 1000.0;
        let cpu = self.last.user + self.last.system;
        json!({"wall_ms":wall,"samples":self.n,"peak_rss_bytes":p("VmRSS").max(p("VmHWM"))*1024,"final_rss_bytes":l("VmRSS")*1024,
            "peak_rss_anon_bytes":p("RssAnon")*1024,"peak_rss_file_bytes":p("RssFile")*1024,"peak_pss_bytes":p("Pss")*1024,
            "peak_swap_bytes":p("VmSwap")*1024,"peak_threads":p("Threads"),"peak_fds":p("fds"),"cpu_user_s":self.last.user,"cpu_sys_s":self.last.system,
            "cpu_total_s":cpu,"cpu_pct":if wall>0.0{cpu*100000.0/wall}else{0.0},"minflt":l("minflt"),"majflt":l("majflt"),
            "vol_ctxt":l("voluntary_ctxt_switches"),"nonvol_ctxt":l("nonvoluntary_ctxt_switches"),"io_read_bytes":l("read_bytes"),"io_write_bytes":l("write_bytes")})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_current_process_and_ignores_missing_process() {
        let mut m = Monitor::new(std::process::id());
        assert!(m.poll());
        let r = m.report();
        assert!(r["peak_rss_bytes"].as_u64().unwrap() > 0);
        assert!(r["peak_threads"].as_u64().unwrap() > 0);
        assert!(!Monitor::new(u32::MAX).poll());
    }
    #[test]
    fn absolute_cpu_and_memory_units() {
        let mut m = Monitor::new(0);
        m.last.user = 1.25;
        m.last.system = 0.5;
        m.last.values.insert("minflt".into(), 40);
        m.peak.insert("VmHWM".into(), 1024);
        let r = m.report();
        assert_eq!(r["cpu_total_s"], 1.75);
        assert_eq!(r["peak_rss_bytes"], 1048576);
        assert_eq!(r["minflt"], 40);
    }
}
