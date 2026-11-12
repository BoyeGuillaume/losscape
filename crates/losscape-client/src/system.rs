//! System metrics, sampled on a background thread and logged as `system/...` metrics.
//! Their step is the unix time in seconds, so they line up with wall time and stay unique
//! across resumes. NVIDIA GPUs are read through NVML, loaded at runtime: machines without
//! the driver simply have no GPU metrics.

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, System};

const FIRST_SAMPLE: Duration = Duration::from_secs(1);

/// NVML, initialized once per process (None without an NVIDIA driver).
pub fn nvml() -> Option<&'static Nvml> {
    static NVML: OnceLock<Option<Nvml>> = OnceLock::new();
    NVML.get_or_init(|| Nvml::init().ok()).as_ref()
}

/// Stops the sampling thread when dropped (or with [`Sampler::stop`]).
pub struct Sampler {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Calls `push(points, time_ms, step)` every `interval`.
    pub fn start(
        interval: Duration,
        push: impl Fn(&[(String, f64)], i64, u64) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let thread = std::thread::Builder::new()
            .name("losscape-system".into())
            .spawn(move || {
                let mut probe = Probe::new();
                let (lock, cvar) = &*signal;
                let mut stopped = lock.lock().expect("not poisoned");
                // The first sample comes early, so short runs have one too (usage figures
                // need some time between readings).
                let mut wait = interval.min(FIRST_SAMPLE);
                loop {
                    stopped = cvar
                        .wait_timeout_while(stopped, wait, |s| !*s)
                        .expect("not poisoned")
                        .0;
                    if *stopped {
                        return;
                    }
                    wait = interval;
                    let points = probe.sample();
                    let now = crate::run::now_ms();
                    push(&points, now, (now / 1000) as u64);
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) {
        self.signal();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }

    fn signal(&self) {
        let (lock, cvar) = &*self.stop;
        *lock.lock().expect("not poisoned") = true;
        cvar.notify_all();
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.signal();
    }
}

struct Probe {
    system: System,
    pid: Option<Pid>,
    disks: Disks,
    networks: Networks,
    last: Instant,
}

impl Probe {
    fn new() -> Self {
        let mut system = System::new();
        let pid = sysinfo::get_current_pid().ok();
        // Usage figures are deltas: take a first reading now.
        system.refresh_cpu_usage();
        if let Some(pid) = pid {
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
        }
        Self {
            system,
            pid,
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            last: Instant::now(),
        }
    }

    fn sample(&mut self) -> Vec<(String, f64)> {
        let seconds = self.last.elapsed().as_secs_f64().max(1e-3);
        self.last = Instant::now();
        let mut out: Vec<(String, f64)> = Vec::new();
        let mut put = |key: &str, value: f64| {
            if value.is_finite() {
                out.push((format!("system/{key}"), value));
            }
        };

        let s = &mut self.system;
        s.refresh_cpu_usage();
        s.refresh_memory();
        // Names say what is measured, not in what: units come with the metric's settings
        // (percent, bytes, watts...), so values can be shown at any scale.
        put("cpu/util", s.global_cpu_usage() as f64);
        if s.total_memory() > 0 {
            put(
                "memory/usage",
                100.0 * s.used_memory() as f64 / s.total_memory() as f64,
            );
        }
        if let Some(pid) = self.pid {
            s.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
            if let Some(p) = s.process(pid) {
                put("cpu/process", p.cpu_usage() as f64);
                put("memory/process", p.memory() as f64); // bytes
                let io = p.disk_usage();
                // Bytes per second (the UI scales them).
                put("disk/read", io.read_bytes as f64 / seconds);
                put("disk/write", io.written_bytes as f64 / seconds);
            }
        }

        self.disks.refresh(false);
        let cwd = std::env::current_dir().unwrap_or_default();
        if let Some(disk) = disk_of(&self.disks, &cwd)
            && disk.total_space() > 0
        {
            let used = disk.total_space() - disk.available_space();
            put(
                "disk/usage",
                100.0 * used as f64 / disk.total_space() as f64,
            );
        }

        self.networks.refresh(false);
        let (mut recv, mut sent) = (0u64, 0u64);
        for (name, data) in &self.networks {
            if name != "lo" && !name.starts_with("lo0") {
                recv += data.received();
                sent += data.transmitted();
            }
        }
        // Bits per second, like network speeds are quoted.
        put("network/recv", recv as f64 * 8.0 / seconds);
        put("network/sent", sent as f64 * 8.0 / seconds);

        if let Some(nvml) = nvml() {
            for i in 0..nvml.device_count().unwrap_or(0) {
                let Ok(gpu) = nvml.device_by_index(i) else {
                    continue;
                };
                if let Ok(u) = gpu.utilization_rates() {
                    put(&format!("gpu{i}/util"), u.gpu as f64);
                }
                if let Ok(m) = gpu.memory_info()
                    && m.total > 0
                {
                    put(
                        &format!("gpu{i}/memory_usage"),
                        100.0 * m.used as f64 / m.total as f64,
                    );
                    put(&format!("gpu{i}/memory"), m.used as f64); // bytes
                }
                if let Ok(mw) = gpu.power_usage() {
                    put(&format!("gpu{i}/power"), mw as f64 / 1000.0); // watts
                }
                if let Ok(c) = gpu.temperature(TemperatureSensor::Gpu) {
                    put(&format!("gpu{i}/temp"), c as f64); // °C
                }
            }
        }
        out
    }
}

/// The disk holding `path`: the longest mount point it is under.
fn disk_of<'a>(disks: &'a Disks, path: &Path) -> Option<&'a sysinfo::Disk> {
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_cpu_memory_and_network() {
        let mut probe = Probe::new();
        std::thread::sleep(Duration::from_millis(250));
        let points = probe.sample();
        let keys: Vec<&str> = points.iter().map(|(k, _)| k.as_str()).collect();
        for key in [
            "system/cpu/util",
            "system/memory/usage",
            "system/memory/process",
            "system/network/recv",
        ] {
            assert!(keys.contains(&key), "{key} missing from {keys:?}");
        }
        let memory = points
            .iter()
            .find(|(k, _)| k == "system/memory/usage")
            .unwrap()
            .1;
        assert!((0.0..=100.0).contains(&memory));
    }

    #[test]
    fn sampler_pushes_and_stops() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let sampler = Sampler::start(Duration::from_millis(50), move |points, _, step| {
            sink.lock().unwrap().push((points.len(), step));
        })
        .unwrap();
        // Waiting for samples, not for a set time: starting (listing processes, disks,
        // networks, loading NVML) can take longer than a few intervals on a busy machine.
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.lock().unwrap().len() < 2 {
            let samples = got.lock().unwrap().len();
            assert!(Instant::now() < deadline, "{samples} samples after 10 s");
            std::thread::sleep(Duration::from_millis(10));
        }
        sampler.stop();
        let samples = got.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(got.lock().unwrap().len(), samples, "stopped");
    }
}
