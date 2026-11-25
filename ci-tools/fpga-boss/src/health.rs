// Licensed under the Apache-2.0 license

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::{ErrorKind, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const FPGA_INFO_PREFIX: &str = "CALIPTRA_FPGA_INFO ";
const MAX_HTTP_RESPONSE_BYTES: usize = 64 * 1024;
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FpgaInfo {
    pub ip: String,
    pub health_port: u16,
    pub variant: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub healthy: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub build_commit: String,
    #[serde(default)]
    pub uptime_secs: Option<f64>,
    #[serde(default)]
    pub loadavg_1m: Option<f64>,
    #[serde(default)]
    pub mem_total_mb: Option<u64>,
    #[serde(default)]
    pub mem_available_mb: Option<u64>,
    #[serde(default)]
    pub root_total_mb: Option<u64>,
    #[serde(default)]
    pub root_free_mb: Option<u64>,
    #[serde(default)]
    pub kernel_tainted: u64,
    #[serde(default)]
    pub net_end0_carrier: i32,
    #[serde(default)]
    pub fpga_manager_state: Option<String>,
    #[serde(default)]
    pub uio_devices: Vec<String>,
    #[serde(default)]
    pub soc_temp_c: Option<f64>,
    #[serde(default)]
    pub runner_listener_alive: bool,
    #[serde(default)]
    pub runner_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardStatusRecord {
    pub identifier: String,
    pub state: String,
    pub updated_at_epoch_secs: u64,
    pub fpga: Option<FpgaInfo>,
    pub health: Option<HealthSnapshot>,
    pub http_status: Option<u16>,
    pub consecutive_health_failures: u32,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusServerResponse {
    pub hostname: String,
    pub timestamp_epoch_secs: u64,
    pub boards: Vec<BoardStatusRecord>,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Resolves the shared fpga-boss state directory.
///
/// Checks `CALIPTRA_FPGA_BOSS_STATE_DIR` first, then `$HOME/.local/state/fpga-boss`
/// so systemd system services (`HOME=/home/hostrunner`, no `XDG_RUNTIME_DIR`) and
/// interactive SSH sessions (`XDG_RUNTIME_DIR=/run/user/1000`) always see the same directory.
pub fn state_dir() -> Option<PathBuf> {
    resolve_state_dir(
        std::env::var_os("CALIPTRA_FPGA_BOSS_STATE_DIR").as_deref(),
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    )
}

fn resolve_state_dir(
    override_dir: Option<&OsStr>,
    home_dir: Option<&OsStr>,
    xdg_runtime_dir: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(home) = home_dir.filter(|d| !d.is_empty()) {
        return Some(Path::new(home).join(".local/state/fpga-boss"));
    }
    xdg_runtime_dir
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("fpga-boss"))
}

pub fn parse_fpga_info_line(line: &str) -> Option<FpgaInfo> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let (_, json_part) = trimmed.split_once(FPGA_INFO_PREFIX)?;
    serde_json::from_str::<FpgaInfo>(json_part.trim()).ok()
}

pub fn fetch_healthz(
    ip: &str,
    port: u16,
    timeout: Duration,
) -> anyhow::Result<(u16, HealthSnapshot)> {
    let ip_addr: IpAddr = ip
        .trim()
        .parse()
        .with_context(|| format!("invalid FPGA IP {:?}", ip))?;
    let sock_addr = SocketAddr::new(ip_addr, port);

    let mut stream = TcpStream::connect_timeout(&sock_addr, timeout)
        .with_context(|| format!("connecting to {sock_addr}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let req = format!(
        "GET /healthz HTTP/1.1\r\nHost: {sock_addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .with_context(|| format!("sending /healthz request to {sock_addr}"))?;

    let mut raw = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if raw.len() + n > MAX_HTTP_RESPONSE_BYTES {
                    anyhow::bail!(
                        "healthz response from {sock_addr} exceeded {MAX_HTTP_RESPONSE_BYTES} bytes"
                    );
                }
                raw.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("reading /healthz response from {sock_addr}"))
            }
        }
    }

    let (header_bytes, body_bytes) = split_http_response(&raw)
        .ok_or_else(|| anyhow!("malformed HTTP response from {sock_addr}"))?;
    let header_str = std::str::from_utf8(header_bytes)
        .with_context(|| format!("non-UTF8 HTTP headers from {sock_addr}"))?;
    let status_line = header_str
        .lines()
        .next()
        .ok_or_else(|| anyhow!("missing HTTP status line from {sock_addr}"))?;
    let status_code = parse_http_status_code(status_line)
        .with_context(|| format!("parsing HTTP status line {:?} from {sock_addr}", status_line))?;

    let snapshot: HealthSnapshot = serde_json::from_slice(body_bytes).with_context(|| {
        format!(
            "decoding /healthz JSON from {sock_addr} (HTTP {status_code}): {}",
            String::from_utf8_lossy(body_bytes)
        )
    })?;

    Ok((status_code, snapshot))
}

fn split_http_response(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((&raw[..pos], &raw[pos + 4..]));
    }
    if let Some(pos) = raw.windows(2).position(|w| w == b"\n\n") {
        return Some((&raw[..pos], &raw[pos + 2..]));
    }
    None
}

fn parse_http_status_code(status_line: &str) -> anyhow::Result<u16> {
    let mut parts = status_line.split_whitespace();
    let proto = parts
        .next()
        .ok_or_else(|| anyhow!("empty HTTP status line"))?;
    if !proto.starts_with("HTTP/") {
        anyhow::bail!("unexpected HTTP protocol token {proto:?}");
    }
    let code_str = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP status code"))?;
    code_str
        .parse::<u16>()
        .with_context(|| format!("invalid HTTP status code {code_str:?}"))
}

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("state");
    let seq = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(".{name}.tmp.{}.{seq}", std::process::id()));

    let write_res = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(contents)?;
        f.sync_all()?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if write_res.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_res
}

pub struct StatusTracker {
    state_dir: Option<PathBuf>,
    record: BoardStatusRecord,
    stopped_cleanly: bool,
}

impl StatusTracker {
    pub fn new(identifier: impl Into<String>) -> Self {
        Self::with_state_dir(identifier, state_dir())
    }

    pub fn with_state_dir(identifier: impl Into<String>, state_dir: Option<PathBuf>) -> Self {
        Self {
            state_dir,
            record: BoardStatusRecord {
                identifier: identifier.into(),
                state: "STARTING".to_string(),
                updated_at_epoch_secs: now_epoch_secs(),
                fpga: None,
                health: None,
                http_status: None,
                consecutive_health_failures: 0,
                last_error: None,
            },
            stopped_cleanly: false,
        }
    }

    pub fn fpga_info(&self) -> Option<&FpgaInfo> {
        self.record.fpga.as_ref()
    }

    pub fn consecutive_health_failures(&self) -> u32 {
        self.record.consecutive_health_failures
    }

    #[cfg(test)]
    pub fn record(&self) -> &BoardStatusRecord {
        &self.record
    }

    pub fn start_new_cycle(&mut self) {
        self.record.fpga = None;
        self.record.health = None;
        self.record.http_status = None;
        self.record.consecutive_health_failures = 0;
    }

    pub fn set_state(&mut self, state: &str) {
        self.record.state = state.to_string();
        self.record.updated_at_epoch_secs = now_epoch_secs();
        self.persist();
    }

    pub fn clear_error_and_set_state(&mut self, state: &str) {
        self.record.last_error = None;
        self.set_state(state);
    }

    pub fn set_error(&mut self, state: &str, err: impl Into<String>) {
        self.record.last_error = Some(err.into());
        self.set_state(state);
    }

    pub fn set_fpga_info(&mut self, info: FpgaInfo) {
        self.record.fpga = Some(info);
        self.record.updated_at_epoch_secs = now_epoch_secs();
        self.persist();
    }

    pub fn record_health_ok(&mut self, http_status: u16, snapshot: HealthSnapshot) {
        self.record.http_status = Some(http_status);
        self.record.health = Some(snapshot);
        self.record.consecutive_health_failures = 0;
        self.record.last_error = None;
        self.record.updated_at_epoch_secs = now_epoch_secs();
        self.persist();
    }

    pub fn record_health_failure(
        &mut self,
        http_status: Option<u16>,
        snapshot: Option<HealthSnapshot>,
        reason: impl Into<String>,
    ) {
        self.record.http_status = http_status;
        if let Some(snap) = snapshot {
            self.record.health = Some(snap);
        }
        self.record.consecutive_health_failures =
            self.record.consecutive_health_failures.saturating_add(1);
        self.record.last_error = Some(reason.into());
        self.record.updated_at_epoch_secs = now_epoch_secs();
        self.persist();
    }

    pub fn mark_stopped_cleanly(&mut self) {
        self.stopped_cleanly = true;
    }

    fn persist(&self) {
        let Some(ref dir) = self.state_dir else {
            return;
        };
        if let Err(e) = fs::create_dir_all(dir) {
            eprintln!("Failed to create status directory {:?}: {e}", dir);
            return;
        }
        let status_path = dir.join(format!("{}.status", self.record.identifier));
        if let Err(e) = atomic_write(&status_path, self.record.state.as_bytes()) {
            eprintln!("Failed to write status file {:?}: {e}", status_path);
        }
        let json_path = dir.join(format!("{}.json", self.record.identifier));
        match serde_json::to_vec_pretty(&self.record) {
            Ok(bytes) => {
                if let Err(e) = atomic_write(&json_path, &bytes) {
                    eprintln!("Failed to write JSON status file {:?}: {e}", json_path);
                }
            }
            Err(e) => {
                eprintln!("Failed to serialize status record {:?}: {e}", json_path);
            }
        }
    }
}

impl Drop for StatusTracker {
    fn drop(&mut self) {
        if !self.stopped_cleanly {
            self.set_state("OFFLINE");
        }
    }
}

pub fn load_board_statuses(dir: &Path, live: bool) -> anyhow::Result<Vec<BoardStatusRecord>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records: BTreeMap<String, BoardStatusRecord> = BTreeMap::new();
    let mut legacy_status_files: Vec<(String, PathBuf)> = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("reading state dir {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if fname.starts_with('.') {
            continue;
        }
        if let Some(stem) = fname.strip_suffix(".json") {
            match fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<BoardStatusRecord>(&bytes) {
                    Ok(rec) => {
                        records.insert(stem.to_string(), rec);
                    }
                    Err(e) => {
                        records.insert(
                            stem.to_string(),
                            BoardStatusRecord {
                                identifier: stem.to_string(),
                                state: "CORRUPT_STATE".to_string(),
                                updated_at_epoch_secs: file_mtime_epoch_secs(&path),
                                fpga: None,
                                health: None,
                                http_status: None,
                                consecutive_health_failures: 0,
                                last_error: Some(format!("failed to parse {:?}: {e}", path)),
                            },
                        );
                    }
                },
                Err(e) => {
                    eprintln!("Warning: failed to read {:?}: {e}", path);
                }
            }
        } else if let Some(stem) = fname.strip_suffix(".status") {
            legacy_status_files.push((stem.to_string(), path));
        }
    }

    for (stem, path) in legacy_status_files {
        if records.contains_key(&stem) {
            continue;
        }
        if let Ok(state_str) = fs::read_to_string(&path) {
            records.insert(
                stem.clone(),
                BoardStatusRecord {
                    identifier: stem,
                    state: state_str.trim().to_string(),
                    updated_at_epoch_secs: file_mtime_epoch_secs(&path),
                    fpga: None,
                    health: None,
                    http_status: None,
                    consecutive_health_failures: 0,
                    last_error: None,
                },
            );
        }
    }

    let mut out: Vec<BoardStatusRecord> = records.into_values().collect();
    out.sort_by(|a, b| a.identifier.cmp(&b.identifier));

    if live {
        refresh_live_health(&mut out);
    }

    Ok(out)
}

fn file_mtime_epoch_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn refresh_live_health(records: &mut [BoardStatusRecord]) {
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (idx, rec) in records.iter().enumerate() {
            if let Some(ref fpga) = rec.fpga {
                if !fpga.ip.trim().is_empty() {
                    let ip = fpga.ip.clone();
                    let port = fpga.health_port;
                    handles.push(scope.spawn(move || {
                        (
                            idx,
                            fetch_healthz(&ip, port, Duration::from_secs(2)),
                        )
                    }));
                }
            }
        }

        for handle in handles {
            if let Ok((idx, res)) = handle.join() {
                let rec = &mut records[idx];
                match res {
                    Ok((code, snap)) => {
                        rec.http_status = Some(code);
                        if snap.healthy {
                            rec.last_error = None;
                        } else {
                            rec.last_error = Some(format!(
                                "live healthz unhealthy (HTTP {code}): {:?}",
                                snap.reasons
                            ));
                        }
                        rec.health = Some(snap);
                    }
                    Err(e) => {
                        rec.http_status = None;
                        rec.health = None;
                        rec.last_error = Some(format!("live healthz check failed: {e}"));
                    }
                }
            }
        }
    });
}

pub fn build_status_response(dir: Option<&Path>, live: bool) -> anyhow::Result<StatusServerResponse> {
    let boards = match dir {
        Some(d) => load_board_statuses(d, live)?,
        None => Vec::new(),
    };
    Ok(StatusServerResponse {
        hostname: read_hostname(),
        timestamp_epoch_secs: now_epoch_secs(),
        boards,
    })
}

pub fn run_health_command(ip: &str, port: u16, json_out: bool) -> anyhow::Result<()> {
    let (status_code, snap) = fetch_healthz(ip, port, Duration::from_secs(3))?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&snap)?);
    } else {
        println!(
            "FPGA {}:{} — {} (HTTP {})",
            ip,
            port,
            if snap.healthy { "HEALTHY" } else { "UNHEALTHY" },
            status_code
        );
        println!(
            "  hostname={} variant={} mode={} commit={}",
            snap.hostname, snap.variant, snap.mode, snap.build_commit
        );
        if !snap.reasons.is_empty() {
            println!("  reasons={}", snap.reasons.join(", "));
        }
        println!(
            "  uptime={} load1m={} mem={}/{} MB root_free={}/{} MB temp={}C",
            fmt_opt_f64(snap.uptime_secs, "s"),
            fmt_opt_f64(snap.loadavg_1m, ""),
            fmt_opt_u64(snap.mem_available_mb),
            fmt_opt_u64(snap.mem_total_mb),
            fmt_opt_u64(snap.root_free_mb),
            fmt_opt_u64(snap.root_total_mb),
            fmt_opt_f64(snap.soc_temp_c, ""),
        );
        println!(
            "  carrier={} fpga_mgr={} uio={:?} tainted={:#x} listener={} busy={}",
            snap.net_end0_carrier,
            snap.fpga_manager_state.as_deref().unwrap_or("-"),
            snap.uio_devices,
            snap.kernel_tainted,
            snap.runner_listener_alive,
            snap.runner_busy,
        );
    }

    if !snap.healthy || status_code != 200 {
        anyhow::bail!(
            "FPGA {}:{} reported unhealthy (HTTP {}): {:?}",
            ip,
            port,
            status_code,
            snap.reasons
        );
    }
    Ok(())
}

pub fn run_status_command(live: bool, json_out: bool) -> anyhow::Result<()> {
    let dir = state_dir();
    let resp = build_status_response(dir.as_deref(), live)?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.boards.is_empty() {
        let loc = dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unset>".to_string());
        println!("No FPGA boss state files found in {loc}");
        return Ok(());
    }

    println!(
        "Host: {} ({} board(s))",
        resp.hostname,
        resp.boards.len()
    );
    println!(
        "{:<22} {:<22} {:<6} {:<16} {:<10} {:<10} {:<9} {:<10} {:<7} {}",
        "IDENTIFIER", "STATE", "AGE", "IP", "VARIANT", "HEALTH", "RUNNER", "MEM_AVAIL", "TEMP", "DETAIL"
    );
    let now = resp.timestamp_epoch_secs;
    for b in &resp.boards {
        let age = if b.updated_at_epoch_secs > 0 && now >= b.updated_at_epoch_secs {
            format_age(now - b.updated_at_epoch_secs)
        } else {
            "-".to_string()
        };
        let ip = b
            .fpga
            .as_ref()
            .map(|f| f.ip.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("-");
        let variant = b
            .fpga
            .as_ref()
            .map(|f| f.variant.as_str())
            .or_else(|| b.health.as_ref().map(|h| h.variant.as_str()))
            .filter(|s| !s.is_empty())
            .unwrap_or("-");
        let health_str = match (&b.health, b.http_status) {
            (Some(h), Some(code)) if h.healthy => format!("ok({code})"),
            (Some(_), Some(code)) => format!("bad({code})"),
            (Some(h), None) if h.healthy => "ok".to_string(),
            (Some(_), None) => "bad".to_string(),
            (None, Some(code)) => format!("http({code})"),
            (None, None) => "-".to_string(),
        };
        let runner_str = match &b.health {
            Some(h) if h.runner_busy => "busy",
            Some(h) if h.runner_listener_alive => "alive",
            Some(_) => "down",
            None => "-",
        };
        let mem_str = b
            .health
            .as_ref()
            .and_then(|h| h.mem_available_mb)
            .map(|m| format!("{m}M"))
            .unwrap_or_else(|| "-".to_string());
        let temp_str = b
            .health
            .as_ref()
            .and_then(|h| h.soc_temp_c)
            .map(|t| format!("{t:.1}C"))
            .unwrap_or_else(|| "-".to_string());
        let detail = format_board_detail(b);

        println!(
            "{:<22} {:<22} {:<6} {:<16} {:<10} {:<10} {:<9} {:<10} {:<7} {}",
            b.identifier,
            b.state,
            age,
            ip,
            variant,
            health_str,
            runner_str,
            mem_str,
            temp_str,
            detail
        );
    }
    Ok(())
}

fn format_board_detail(b: &BoardStatusRecord) -> String {
    b.last_error
        .clone()
        .or_else(|| {
            b.health
                .as_ref()
                .filter(|h| !h.reasons.is_empty())
                .map(|h| h.reasons.join(","))
        })
        .unwrap_or_default()
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn fmt_opt_f64(v: Option<f64>, suffix: &str) -> String {
    v.map(|n| format!("{n:.2}{suffix}"))
        .unwrap_or_else(|| "-".to_string())
}

fn fmt_opt_u64(v: Option<u64>) -> String {
    v.map(|n| n.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub fn run_status_server(listen_addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr)
        .with_context(|| format!("binding status-server on {listen_addr}"))?;
    eprintln!("caliptra-fpga-boss status-server listening on {listen_addr}");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let dir = state_dir();
                handle_status_client(stream, dir.as_deref());
            }
            Err(e) => {
                eprintln!("status-server accept error: {e}");
            }
        }
    }
    Ok(())
}

pub fn handle_status_client(mut stream: TcpStream, dir: Option<&Path>) {
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
    let raw_target = parts.next().unwrap_or("/");
    let (route, query) = raw_target
        .split_once('?')
        .map_or((raw_target, ""), |(r, q)| (r, q));

    if matches!(route, "/" | "/status" | "/healthz") {
        let live = query
            .split('&')
            .any(|kv| matches!(kv, "live=1" | "live=true"));
        match build_status_response(dir, live)
            .and_then(|resp| serde_json::to_string(&resp).map_err(anyhow::Error::from))
        {
            Ok(mut body) => {
                body.push('\n');
                write_http_json_response(&mut stream, "200 OK", &body, method != "HEAD");
            }
            Err(e) => {
                let body = format!(
                    "{}\n",
                    serde_json::json!({"error": e.to_string()})
                );
                write_http_json_response(
                    &mut stream,
                    "500 Internal Server Error",
                    &body,
                    method != "HEAD",
                );
            }
        }
    } else {
        let body = "{\"error\":\"not found\"}\n";
        write_http_json_response(&mut stream, "404 Not Found", body, method != "HEAD");
    }
}

fn write_http_json_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
    send_body: bool,
) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if send_body {
        let _ = stream.write_all(body.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use tempfile::TempDir;

    fn sample_health_json(healthy: bool, reasons: &[&str], listener_alive: bool) -> String {
        serde_json::json!({
            "healthy": healthy,
            "reasons": reasons,
            "hostname": "caliptra-fpga",
            "mode": "ci",
            "variant": "subsystem",
            "build_commit": "abc1234",
            "uptime_secs": 42.5,
            "loadavg_1m": 0.25,
            "mem_total_mb": 4096,
            "mem_available_mb": 3500,
            "root_total_mb": 3000,
            "root_free_mb": 2900,
            "kernel_tainted": 0,
            "net_end0_carrier": 1,
            "fpga_manager_state": "operating",
            "uio_devices": ["uio0", "uio1"],
            "soc_temp_c": 48.25,
            "runner_listener_alive": listener_alive,
            "runner_busy": false
        })
        .to_string()
    }

    fn spawn_mock_healthd(status: &str, body: String) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_string();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req = [0_u8; 512];
                let _ = stream.read(&mut req);
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (addr, handle)
    }

    #[test]
    fn test_parse_fpga_info_line() {
        assert_eq!(
            parse_fpga_info_line(
                r#"CALIPTRA_FPGA_INFO {"ip":"192.168.1.42","health_port":8080,"variant":"subsystem"}"#
            ),
            Some(FpgaInfo {
                ip: "192.168.1.42".to_string(),
                health_port: 8080,
                variant: "subsystem".to_string(),
            })
        );

        assert_eq!(
            parse_fpga_info_line(
                "[   12.345678] startup-script[123]: CALIPTRA_FPGA_INFO {\"ip\":\"10.0.0.9\",\"health_port\":8080,\"variant\":\"core\"}\r\n"
            ),
            Some(FpgaInfo {
                ip: "10.0.0.9".to_string(),
                health_port: 8080,
                variant: "core".to_string(),
            })
        );

        assert_eq!(
            parse_fpga_info_line(
                r#"CALIPTRA_FPGA_INFO {"ip":"","health_port":8080,"variant":"zcu104"}"#
            ),
            Some(FpgaInfo {
                ip: "".to_string(),
                health_port: 8080,
                variant: "zcu104".to_string(),
            })
        );

        assert_eq!(parse_fpga_info_line("36668aa492b1c83cdd3ade8466a0153d --- Command input"), None);
        assert_eq!(parse_fpga_info_line("CALIPTRA_FPGA_INFO {not valid json"), None);
    }

    #[test]
    fn test_format_board_detail_reasons_fallback() {
        let unhealthy_snap: HealthSnapshot = serde_json::from_str(&sample_health_json(
            false,
            &["low_memory", "end0_link_down"],
            false,
        ))
        .unwrap();
        let healthy_snap: HealthSnapshot =
            serde_json::from_str(&sample_health_json(true, &[], true)).unwrap();

        let mut record = BoardStatusRecord {
            identifier: "caliptra-kir-vck-0".to_string(),
            state: "IDLE".to_string(),
            updated_at_epoch_secs: 100,
            fpga: None,
            health: Some(unhealthy_snap),
            http_status: Some(503),
            consecutive_health_failures: 1,
            last_error: None,
        };
        assert_eq!(format_board_detail(&record), "low_memory,end0_link_down");

        record.last_error = Some("explicit error wins".to_string());
        assert_eq!(format_board_detail(&record), "explicit error wins");

        record.last_error = None;
        record.health = Some(healthy_snap);
        assert_eq!(format_board_detail(&record), "");
    }

    #[test]
    fn test_resolve_state_dir_priority() {
        assert_eq!(
            resolve_state_dir(
                Some(OsStr::new("/tmp/override")),
                Some(OsStr::new("/home/hostrunner")),
                Some(OsStr::new("/run/user/1000")),
            ),
            Some(PathBuf::from("/tmp/override"))
        );
        // HOME must take precedence over XDG_RUNTIME_DIR so systemd services and SSH agree
        assert_eq!(
            resolve_state_dir(
                None,
                Some(OsStr::new("/home/hostrunner")),
                Some(OsStr::new("/run/user/1000")),
            ),
            Some(PathBuf::from("/home/hostrunner/.local/state/fpga-boss"))
        );
        assert_eq!(
            resolve_state_dir(Some(OsStr::new("")), None, Some(OsStr::new("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000/fpga-boss"))
        );
        assert_eq!(resolve_state_dir(None, None, None), None);
    }

    #[test]
    fn test_fetch_healthz_ok_and_503_and_refused() {
        let body_ok = sample_health_json(true, &[], true);
        let (addr_ok, h1) = spawn_mock_healthd("200 OK", body_ok);
        let (code, snap) =
            fetch_healthz(&addr_ok.ip().to_string(), addr_ok.port(), Duration::from_secs(2))
                .unwrap();
        h1.join().unwrap();
        assert_eq!(code, 200);
        assert!(snap.healthy);
        assert!(snap.runner_listener_alive);
        assert_eq!(snap.variant, "subsystem");
        assert_eq!(snap.uio_devices, vec!["uio0", "uio1"]);

        let body_bad = sample_health_json(false, &["low_memory", "end0_link_down"], false);
        let (addr_bad, h2) = spawn_mock_healthd("503 Service Unavailable", body_bad);
        let (code, snap) = fetch_healthz(
            &addr_bad.ip().to_string(),
            addr_bad.port(),
            Duration::from_secs(2),
        )
        .unwrap();
        h2.join().unwrap();
        assert_eq!(code, 503);
        assert!(!snap.healthy);
        assert!(!snap.runner_listener_alive);
        assert_eq!(snap.reasons, vec!["low_memory", "end0_link_down"]);

        // Unused local port -> connection refused
        let unused_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_addr = unused_listener.local_addr().unwrap();
        drop(unused_listener);
        assert!(
            fetch_healthz(
                &closed_addr.ip().to_string(),
                closed_addr.port(),
                Duration::from_millis(500)
            )
            .is_err()
        );
    }

    #[test]
    fn test_status_tracker_load_and_status_server() {
        let dir = TempDir::new().unwrap();
        let health_body = sample_health_json(true, &[], true);
        let (mock_addr, mock_handle) = spawn_mock_healthd("200 OK", health_body);

        {
            let mut tracker =
                StatusTracker::with_state_dir("caliptra-kir-vck-1", Some(dir.path().to_path_buf()));
            tracker.set_state("BOOTING");
            tracker.set_fpga_info(FpgaInfo {
                ip: mock_addr.ip().to_string(),
                health_port: mock_addr.port(),
                variant: "subsystem".to_string(),
            });
            let (_, snap) = fetch_healthz(
                &mock_addr.ip().to_string(),
                mock_addr.port(),
                Duration::from_secs(2),
            )
            .unwrap();
            mock_handle.join().unwrap();
            tracker.record_health_ok(200, snap);
            tracker.clear_error_and_set_state("IDLE");

            // Also add a legacy `.status`-only board file
            fs::write(dir.path().join("caliptra-kir-zcu-0.status"), "BUSY").unwrap();

            let loaded = load_board_statuses(dir.path(), false).unwrap();
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded[0].identifier, "caliptra-kir-vck-1");
            assert_eq!(loaded[0].state, "IDLE");
            assert_eq!(loaded[0].http_status, Some(200));
            assert_eq!(loaded[1].identifier, "caliptra-kir-zcu-0");
            assert_eq!(loaded[1].state, "BUSY");
            assert_eq!(loaded[1].fpga, None);

            // Verify atomic .status & .json exist
            assert_eq!(
                fs::read_to_string(dir.path().join("caliptra-kir-vck-1.status")).unwrap(),
                "IDLE"
            );
            assert!(dir.path().join("caliptra-kir-vck-1.json").exists());

            // Test status-server GET /status and GET /status?live=1
            let live_body = sample_health_json(false, &["low_root_disk"], false);
            let (_live_addr, live_handle) = {
                // Rebind mock healthd on an active port and update tracker IP/port
                let (addr, h) = spawn_mock_healthd("503 Service Unavailable", live_body);
                tracker.set_fpga_info(FpgaInfo {
                    ip: addr.ip().to_string(),
                    health_port: addr.port(),
                    variant: "subsystem".to_string(),
                });
                (addr, h)
            };

            let srv_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let srv_addr = srv_listener.local_addr().unwrap();
            let dir_path = dir.path().to_path_buf();
            let srv_handle = thread::spawn(move || {
                for _ in 0..2 {
                    let (stream, _) = srv_listener.accept().unwrap();
                    handle_status_client(stream, Some(&dir_path));
                }
            });

            // Request 1: cached /status
            let mut c1 = TcpStream::connect(srv_addr).unwrap();
            c1.write_all(b"GET /status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut r1 = String::new();
            c1.read_to_string(&mut r1).unwrap();
            assert!(r1.starts_with("HTTP/1.1 200 OK\r\n"));
            let (_, body1) = r1.split_once("\r\n\r\n").unwrap();
            let parsed1: StatusServerResponse = serde_json::from_str(body1).unwrap();
            assert_eq!(parsed1.boards.len(), 2);
            assert!(parsed1.boards[0].health.as_ref().unwrap().healthy);

            // Request 2: live /status?live=1
            let mut c2 = TcpStream::connect(srv_addr).unwrap();
            c2.write_all(
                b"GET /status?live=1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
            let mut r2 = String::new();
            c2.read_to_string(&mut r2).unwrap();
            srv_handle.join().unwrap();
            live_handle.join().unwrap();
            assert!(r2.starts_with("HTTP/1.1 200 OK\r\n"));
            let (_, body2) = r2.split_once("\r\n\r\n").unwrap();
            let parsed2: StatusServerResponse = serde_json::from_str(body2).unwrap();
            assert_eq!(parsed2.boards[0].http_status, Some(503));
            assert!(!parsed2.boards[0].health.as_ref().unwrap().healthy);
            assert_eq!(
                parsed2.boards[0].health.as_ref().unwrap().reasons,
                vec!["low_root_disk"]
            );

            tracker.mark_stopped_cleanly();
        }
    }
}
