// Licensed under the Apache-2.0 license
//
// Zero-dependency Rust HTTP health agent (`caliptra-fpga-healthd`) for Caliptra NixOS FPGAs.
// Exposes `GET /healthz` on port 8080 (override via `CALIPTRA_HEALTHD_PORT` or
// `CALIPTRA_HEALTHD_LISTEN`) for `fpga-boss` and hostrunner fleet monitoring.

use std::{
    ffi::{c_char, c_int, c_ulong},
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};

#[repr(C)]
struct Statvfs {
    f_bsize: c_ulong,
    f_frsize: c_ulong,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: c_ulong,
    f_flag: c_ulong,
    f_namemax: c_ulong,
    __f_spare: [c_int; 6],
    _pad: [u64; 8],
}

extern "C" {
    fn statvfs(path: *const c_char, buf: *mut Statvfs) -> c_int;
}

fn root_disk_mb() -> Option<(u64, u64)> {
    let path = b"/\0";
    let mut st: Statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { statvfs(path.as_ptr().cast::<c_char>(), &mut st) };
    if rc != 0 {
        return None;
    }
    let unit = if st.f_frsize > 0 {
        st.f_frsize as u64
    } else {
        st.f_bsize as u64
    };
    let total_mb = st.f_blocks.saturating_mul(unit) / (1024 * 1024);
    let free_mb = st.f_bavail.saturating_mul(unit) / (1024 * 1024);
    Some((total_mb, free_mb))
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_meminfo_mb() -> (Option<u64>, Option<u64>) {
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return (None, None);
    };
    let mut total_kb = None;
    let mut avail_kb = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        }
    }
    (total_kb.map(|k| k / 1024), avail_kb.map(|k| k / 1024))
}

fn read_soc_temp_c() -> Option<f64> {
    if let Some(milli) =
        read_trimmed("/sys/class/thermal/thermal_zone0/temp").and_then(|s| s.parse::<f64>().ok())
    {
        return Some(milli / 1000.0);
    }

    let entries = fs::read_dir("/sys/bus/iio/devices").ok()?;
    for entry in entries.flatten() {
        let base = entry.path();
        for prefix in ["in_temp0_temp", "in_temp0", "in_temp160_temp"] {
            let raw = fs::read_to_string(base.join(format!("{prefix}_raw")))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let scale = fs::read_to_string(base.join(format!("{prefix}_scale")))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let offset = fs::read_to_string(base.join(format!("{prefix}_offset")))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0);
            if let (Some(raw), Some(scale)) = (raw, scale) {
                return Some(((raw + offset) * scale) / 1000.0);
            }
        }
    }
    None
}

fn list_uio_devices() -> Vec<String> {
    let mut devs = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/uio") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                devs.push(name.to_string());
            }
        }
    }
    devs.sort();
    devs
}

fn scan_runner_processes() -> (bool, bool) {
    let mut listener = false;
    let mut worker = false;
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let Some(s) = fname.to_str() else { continue };
            if !s.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(comm) = fs::read_to_string(entry.path().join("comm")) {
                match comm.trim() {
                    "Runner.Listener" => listener = true,
                    "Runner.Worker" => worker = true,
                    _ => {}
                }
            }
        }
    }
    (listener, worker)
}

fn read_build_info() -> (String, String, String) {
    let mut mode = "unknown".to_string();
    let mut variant = "unknown".to_string();
    let mut commit = "unknown".to_string();
    if let Ok(text) = fs::read_to_string("/etc/caliptra-build-info") {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                let val = v.trim();
                match key {
                    "Image env" => mode = if val == "dev" { "dev" } else { "ci" }.to_string(),
                    "Image variant" => variant = val.to_string(),
                    "Workflow build commit" => commit = val.to_string(),
                    _ => {}
                }
            }
        }
    }
    (mode, variant, commit)
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn read_wired_carrier() -> i32 {
    for iface in ["end0", "eth0"] {
        let path = format!("/sys/class/net/{iface}/carrier");
        if let Some(val) = read_trimmed(&path).and_then(|s| s.parse::<i32>().ok()) {
            return val;
        }
    }
    -1
}

fn build_health_json() -> (bool, String) {
    let hostname = read_trimmed("/proc/sys/kernel/hostname").unwrap_or_else(|| "unknown".into());
    let (mode, variant, build_commit) = read_build_info();

    let uptime_secs =
        read_trimmed("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
    let loadavg_1m = read_trimmed("/proc/loadavg")
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
    let (mem_total_mb, mem_available_mb) = read_meminfo_mb();
    let root_disk = root_disk_mb();
    let tainted = read_trimmed("/proc/sys/kernel/tainted")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let end0_carrier = read_wired_carrier();
    let fpga_mgr_state = read_trimmed("/sys/class/fpga_manager/fpga0/state");
    let uio_devices = list_uio_devices();
    let soc_temp_c = read_soc_temp_c();
    let (runner_listener_alive, runner_busy) = scan_runner_processes();

    let mut reasons: Vec<&str> = Vec::new();
    // TAINT_DIE (bit 7) or TAINT_SOFTLOCKUP (bit 14); ignore TAINT_OOT_MODULE (bit 12)
    if (tainted & ((1 << 7) | (1 << 14))) != 0 {
        reasons.push("kernel_oops_or_softlockup");
    }
    if let Some(avail) = mem_available_mb {
        if avail < 64 {
            reasons.push("low_memory");
        }
    }
    if let Some((_, free_mb)) = root_disk {
        if free_mb < 64 {
            reasons.push("low_root_disk");
        }
    }
    if end0_carrier == 0 {
        reasons.push("end0_link_down");
    }

    let healthy = reasons.is_empty();

    let reasons_json = reasons
        .iter()
        .map(|r| json_escape(r))
        .collect::<Vec<_>>()
        .join(",");
    let uio_json = uio_devices
        .iter()
        .map(|d| json_escape(d))
        .collect::<Vec<_>>()
        .join(",");
    let opt_u64 = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_else(|| "null".into());
    let opt_f64 = |v: Option<f64>| {
        v.map(|n| format!("{n:.2}"))
            .unwrap_or_else(|| "null".into())
    };
    let fpga_mgr_json = fpga_mgr_state
        .as_deref()
        .map(json_escape)
        .unwrap_or_else(|| "null".into());

    let body = format!(
        "{{\"healthy\":{},\"reasons\":[{}],\"hostname\":{},\"mode\":{},\"variant\":{},\"build_commit\":{},\"uptime_secs\":{},\"loadavg_1m\":{},\"mem_total_mb\":{},\"mem_available_mb\":{},\"root_total_mb\":{},\"root_free_mb\":{},\"kernel_tainted\":{},\"net_end0_carrier\":{},\"fpga_manager_state\":{},\"uio_devices\":[{}],\"soc_temp_c\":{},\"runner_listener_alive\":{},\"runner_busy\":{}}}\n",
        healthy,
        reasons_json,
        json_escape(&hostname),
        json_escape(&mode),
        json_escape(&variant),
        json_escape(&build_commit),
        opt_f64(uptime_secs),
        opt_f64(loadavg_1m),
        opt_u64(mem_total_mb),
        opt_u64(mem_available_mb),
        opt_u64(root_disk.map(|d| d.0)),
        opt_u64(root_disk.map(|d| d.1)),
        tainted,
        end0_carrier,
        fpga_mgr_json,
        uio_json,
        opt_f64(soc_temp_c),
        runner_listener_alive,
        runner_busy,
    );

    (healthy, body)
}

fn handle_client(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let mut req_buf = [0_u8; 1024];
    let n = match stream.read(&mut req_buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let req_line = std::str::from_utf8(&req_buf[..n])
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("");
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    if path == "/" || path == "/healthz" || path.starts_with("/healthz?") {
        let (healthy, body) = build_health_json();
        let status = if healthy {
            "200 OK"
        } else {
            "503 Service Unavailable"
        };
        let header = format!(
            "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status,
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        if method != "HEAD" {
            let _ = stream.write_all(body.as_bytes());
        }
    } else {
        let body = "{\"error\":\"not found\"}\n";
        let header = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        if method != "HEAD" {
            let _ = stream.write_all(body.as_bytes());
        }
    }
}

fn resolve_listen_addr() -> String {
    if let Ok(addr) = std::env::var("CALIPTRA_HEALTHD_LISTEN") {
        if !addr.trim().is_empty() {
            return addr;
        }
    }
    let port = std::env::var("CALIPTRA_HEALTHD_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|&p| p > 0)
        .unwrap_or(8080);
    format!("0.0.0.0:{port}")
}

fn main() {
    let listen_addr = resolve_listen_addr();
    let listener = TcpListener::bind(&listen_addr)
        .unwrap_or_else(|e| panic!("caliptra-fpga-healthd failed to bind {listen_addr}: {e}"));
    eprintln!("caliptra-fpga-healthd (rust) listening on {listen_addr}");

    for incoming in listener.incoming() {
        if let Ok(stream) = incoming {
            handle_client(stream);
        }
    }
}
