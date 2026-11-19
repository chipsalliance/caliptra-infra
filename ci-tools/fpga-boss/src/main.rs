// Licensed under the Apache-2.0 license

mod find_usb_block_device;
mod fpga_jtag;
mod ftdi;
mod ftdi_uart;
mod health;
mod sd_mux;
mod tty;
mod usb_port_path;

use anyhow::{anyhow, Context};

use clap::{arg, value_parser};
use libftdi1_sys::ftdi_interface;
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{stdout, Error, ErrorKind, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

pub(crate) use fpga_jtag::{FpgaJtag, FpgaReset};
pub(crate) use ftdi::FtdiCtx;
use ftdi_uart::{UartEvent, UartLineStream};
use health::{FpgaInfo, StatusTracker};
use sd_mux::{SDWire, SdMux, SdMuxTarget, UsbsdMux};
pub(crate) use usb_port_path::UsbPortPath;

// 3 hours
const JOB_TIMEOUT: Duration = Duration::from_secs(10_800);
const BOOT_TIMEOUT: Duration = Duration::from_secs(180);
const UART_TICK_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const IDLE_HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_RUNNER_GRACE_PERIOD: Duration = Duration::from_secs(20);
const BUSY_HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(30);
const COMMAND_INPUT_SENTINEL: &str = "36668aa492b1c83cdd3ade8466a0153d --- Command input";
const ACTION_COMPLETE_SENTINEL: &str = "3297327285280f1ffb8b57222e0a5033 --- ACTION IS COMPLETE";

fn cli() -> clap::Command<'static> {
    clap::Command::new("fpga-boss")
        .about("FPGA boss tool")
        .arg(
            arg!(--"sdwire" [PORT_PATH] "USB port path to the hub chip on the SDWire (ex: 3-1.2)")
                .value_parser(value_parser!(OsString)))
        .arg(
            arg!(--"usbsdmux" [USBID] "USB unique ID for the usbsdmux device (ex: 00048.00643)")
                .value_parser(value_parser!(OsString))
        )
        .arg(
            arg!(--"zcu104" [PORT_PATH] "USB port path to the FTDI chip on the ZCU104 dev board (ex: 3-1.2)")
                .value_parser(value_parser!(UsbPortPath))
        )
        .arg(
            arg!(--"boss_ftdi" [PORT_PATH] "USB port path to a FTDI C232HM-DDHSL cable plugged into an rPi (ex: 3-1.2")
                .value_parser(value_parser!(UsbPortPath))
        )
        .subcommand_required(true)
        .subcommand(clap::Command::new("mode")
            .about("Set the state of the reset / sdwire pins")
            .arg(arg!(<MODE>).value_parser(value_parser!(SdMuxTarget))))
            .arg_required_else_help(true)
        .subcommand(clap::Command::new("flash")
            .about("Flash an image file to the sdwire and boot the DUT")
            .arg(arg!(-r --read_first).takes_value(false).help("To reduce write cycles, read contents first, and don't flash if they are already as expected."))
            .arg(arg!(<IMAGE_FILENAME>).value_parser(value_parser!(PathBuf))))
        .subcommand(clap::Command::new("console")
            .about("Tail the UART output from zcu104"))
        .subcommand(clap::Command::new("reset_fpga")
            .about("Reset FPGA SoC using FTDI on zcu104 by toggle POR_B reset."))
        .subcommand(clap::Command::new("reset_ftdi")
            .about("Reset FTDI chip on zcu104"))
        .subcommand(clap::Command::new("serve")
            .arg(arg!(<IMAGE_FILENAME>).value_parser(value_parser!(PathBuf)))
            .arg(arg!(<CMD> ...).value_parser(value_parser!(OsString)).help("The command (and arguments) to read jitconfig entries from")))
        .subcommand(clap::Command::new("health")
            .about("Query HTTP /healthz on an FPGA running caliptra-fpga-healthd")
            .arg(arg!(<IP> "IP address of the FPGA").value_parser(value_parser!(String)))
            .arg(arg!(--"port" [PORT] "Port caliptra-fpga-healthd listens on")
                .default_value("8080")
                .value_parser(value_parser!(u16)))
            .arg(arg!(--"json" "Print raw JSON").takes_value(false)))
        .subcommand(clap::Command::new("status")
            .about("Show health and runner state of all fpga-boss instances on this host")
            .arg(arg!(--"live" "Query live /healthz from each board with a known IP").takes_value(false))
            .arg(arg!(--"json" "Print JSON output").takes_value(false)))
        .subcommand(clap::Command::new("status-server")
            .about("Serve HTTP JSON status across all fpga-boss instances on this host")
            .arg(arg!(--"listen" [ADDR] "Socket address to bind")
                .default_value("0.0.0.0:8081")
                .value_parser(value_parser!(String))))
}

fn main() {
    match main_impl() {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Fatal error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn open_block_dev(path: &Path) -> std::io::Result<File> {
    let mut tries = 0_u32;
    loop {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(result) => return Ok(result),
            Err(err) => {
                if err.raw_os_error() == Some(libc::ENOMEDIUM) {
                    if tries == 0 {
                        println!("Waiting for attached sd card to be noticed by OS")
                    }
                    // SD card hasn't been found by the OS yet
                    tries += 1;
                    if tries < 1000 {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }
                return Err(err);
            }
        }
    }
}

/// Returns true if the device already contains the image.
fn verify_image(dev: &mut File, image: &mut File) -> std::io::Result<bool> {
    dev.seek(SeekFrom::Start(0))?;
    let file_len = image.metadata()?.len();
    let mut want = vec![0_u8; 1024 * 1024];
    let mut have = vec![0_u8; 1024 * 1024];
    let mut total_read: u64 = 0;
    let start_time = Instant::now();
    loop {
        let bytes_read = image.read(&mut want)?;
        if bytes_read == 0 {
            return Ok(true);
        }
        dev.read_exact(&mut have[..bytes_read])?;
        if want[..bytes_read] != have[..bytes_read] {
            return Ok(false);
        }
        total_read += u64::try_from(bytes_read).unwrap();
        let duration = Instant::now() - start_time;
        print!(
            "Read {} MB of {} MB: {:.1} MB/sec \r",
            total_read / (1024 * 1024),
            file_len / (1024 * 1024),
            total_read as f64 / duration.as_secs_f64() / (1024.0 * 1024.0)
        );
        std::io::stdout().flush()?;
    }
}

fn copy_file(dest: &mut File, src: &mut File) -> std::io::Result<()> {
    src.seek(SeekFrom::Start(0))?;
    dest.seek(SeekFrom::Start(0))?;
    let file_len = src.metadata()?.len();
    let mut buf = vec![0_u8; 1024 * 1024];
    let mut total_written: u64 = 0;
    let start_time = Instant::now();
    loop {
        let bytes_read = src.read(&mut buf)?;
        if bytes_read == 0 {
            break;
        }
        total_written += u64::try_from(bytes_read).unwrap();
        dest.write_all(&buf[..bytes_read])?;
        dest.sync_data()?;
        let duration = Instant::now() - start_time;
        print!(
            "Wrote {} MB of {} MB: {:.1} MB/sec \r",
            total_written / (1024 * 1024),
            file_len / (1024 * 1024),
            total_written as f64 / duration.as_secs_f64() / (1024.0 * 1024.0)
        );
        std::io::stdout().flush()?;
    }
    dest.flush()?;
    dest.sync_all()?;
    Ok(())
}

/// As we observe conditions that get the FGPA stuck, add them to the error parser.
fn active_runner_error_checks(input: &str) -> std::io::Result<()> {
    check_for_github_runner_exception(input)?;
    check_for_kernel_panic(input)?;
    Ok(())
}

/// If the GitHub runner has an unhandled exception it will crash and get stuck. Add this check
/// to recover the FPGA.
fn check_for_github_runner_exception(input: &str) -> std::io::Result<()> {
    if input.contains("Unhandled exception") || input.contains("Bus error") {
        Err(Error::new(
            ErrorKind::BrokenPipe,
            "Github runner had an unhandled exception",
        ))?;
    }
    Ok(())
}

/// A kernel panic will cause the FPGA to never complete the job, so we want to reset the FPGA.
fn check_for_kernel_panic(input: &str) -> std::io::Result<()> {
    if input.contains("Kernel panic") {
        Err(Error::new(ErrorKind::BrokenPipe, "FPGA had a kernel panic"))?;
    }
    Ok(())
}

fn poll_health_tick(
    tracker: &mut StatusTracker,
    info: &FpgaInfo,
    phase: &str,
    require_runner_alive: bool,
) -> bool {
    match health::fetch_healthz(&info.ip, info.health_port, HEALTH_HTTP_TIMEOUT) {
        Ok((http_status, snap)) => {
            let runner_ok =
                !require_runner_alive || snap.runner_listener_alive || snap.runner_busy;
            if snap.healthy && runner_ok {
                tracker.record_health_ok(http_status, snap);
                true
            } else {
                let reason = if !snap.healthy {
                    format!(
                        "{phase} healthz unhealthy (HTTP {http_status}): {:?}",
                        snap.reasons
                    )
                } else {
                    format!("{phase} runner_listener_not_alive")
                };
                eprintln!("{reason}");
                tracker.record_health_failure(Some(http_status), Some(snap), reason);
                false
            }
        }
        Err(e) => {
            let reason = format!("{phase} healthz poll failed: {e}");
            eprintln!("{reason}");
            tracker.record_health_failure(None, None, reason);
            false
        }
    }
}

fn wait_for_boot<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
    timeout: Duration,
) -> anyhow::Result<bool> {
    let start = Instant::now();
    let mut dhcp_failed = false;
    loop {
        if start.elapsed() >= timeout {
            eprintln!("Timed out waiting for FPGA to boot!");
            tracker.set_error("BOOT_TIMEOUT", "Timed out waiting for FPGA to boot");
            return Ok(false);
        }
        match uart_stream.next_event()? {
            UartEvent::Line(line) => {
                println!("UART: {}", line);
                if let Some(info) = health::parse_fpga_info_line(&line) {
                    if info.ip.trim().is_empty() {
                        eprintln!("FPGA reported empty IP in CALIPTRA_FPGA_INFO (DHCP failed)");
                        dhcp_failed = true;
                        tracker.set_error(
                            "DHCP_FAILED",
                            "FPGA reported empty IP in CALIPTRA_FPGA_INFO",
                        );
                    } else {
                        tracker.set_fpga_info(info);
                    }
                }
                if line.contains(COMMAND_INPUT_SENTINEL) {
                    return Ok(!dhcp_failed);
                }
            }
            UartEvent::TickTimeout => {}
            UartEvent::Eof => {
                return Err(anyhow!("unexpected EOF from FPGA UART while booting"));
            }
        }
    }
}

fn verify_pre_job_health(tracker: &mut StatusTracker) -> bool {
    let Some(info) = tracker.fpga_info().cloned() else {
        println!("No CALIPTRA_FPGA_INFO received on UART; proceeding in legacy UART-only mode");
        return true;
    };

    for attempt in 1..=3 {
        if poll_health_tick(tracker, &info, &format!("pre-job ({attempt}/3)"), false) {
            return true;
        }
        if attempt < 3 {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    tracker.set_state("UNHEALTHY");
    false
}

fn wait_for_registration<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
    timeout: Duration,
) -> anyhow::Result<bool> {
    let start = Instant::now();
    loop {
        if start.elapsed() >= timeout {
            eprintln!("Timed out waiting for FPGA to connect to Github!");
            tracker.set_error(
                "GITHUB_CONNECT_TIMEOUT",
                "Timed out waiting for FPGA to connect to GitHub",
            );
            return Ok(false);
        }
        match uart_stream.next_event()? {
            UartEvent::Line(line) => {
                println!("UART: {}", line);
                if line.contains("Listening for Jobs") {
                    return Ok(true);
                }
                if line.contains(COMMAND_INPUT_SENTINEL) {
                    eprintln!("FPGA rebooted unexpectedly while registering!");
                    tracker.set_error(
                        "UNEXPECTED_REBOOT",
                        "FPGA rebooted unexpectedly while registering",
                    );
                    return Ok(false);
                }
                if let Err(e) = active_runner_error_checks(&line) {
                    eprintln!("Runner error while registering: {e}");
                    tracker.set_error("RUNNER_ERROR", e.to_string());
                    return Ok(false);
                }
            }
            UartEvent::TickTimeout => {}
            UartEvent::Eof => {
                return Err(anyhow!("unexpected EOF from FPGA UART while registering"));
            }
        }
    }
}

fn wait_for_job_start<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
) -> anyhow::Result<bool> {
    wait_for_job_start_with(
        uart_stream,
        tracker,
        IDLE_HEALTH_POLL_INTERVAL,
        IDLE_RUNNER_GRACE_PERIOD,
    )
}

fn wait_for_job_start_with<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
    poll_interval: Duration,
    grace_period: Duration,
) -> anyhow::Result<bool> {
    tracker.clear_error_and_set_state("IDLE");
    let idle_start = Instant::now();
    let mut last_health_poll = Instant::now();
    loop {
        match uart_stream.next_event()? {
            UartEvent::Line(line) => {
                println!("UART: {}", line);
                if line.contains("Running job") {
                    return Ok(true);
                }
                if let Some(info) = health::parse_fpga_info_line(&line) {
                    if !info.ip.trim().is_empty() {
                        tracker.set_fpga_info(info);
                    }
                }
                if line.contains(COMMAND_INPUT_SENTINEL) {
                    eprintln!("FPGA rebooted unexpectedly while IDLE!");
                    tracker.set_error("UNEXPECTED_REBOOT", "FPGA rebooted unexpectedly while IDLE");
                    return Ok(false);
                }
                if let Err(e) = active_runner_error_checks(&line) {
                    eprintln!("Runner error while IDLE: {e}");
                    tracker.set_error("RUNNER_ERROR", e.to_string());
                    return Ok(false);
                }
            }
            UartEvent::TickTimeout => {}
            UartEvent::Eof => {
                return Err(anyhow!("unexpected EOF from FPGA UART while IDLE"));
            }
        }

        if last_health_poll.elapsed() >= poll_interval {
            if let Some(info) = tracker.fpga_info().cloned() {
                last_health_poll = Instant::now();
                let require_runner = idle_start.elapsed() >= grace_period;
                poll_health_tick(tracker, &info, "IDLE", require_runner);
                if tracker.consecutive_health_failures() >= 2 {
                    eprintln!("FPGA failed 2 consecutive IDLE health checks; resetting");
                    tracker.set_state("UNHEALTHY");
                    return Ok(false);
                }
            }
        }
    }
}

fn wait_for_job_complete<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
    timeout: Duration,
) -> anyhow::Result<bool> {
    wait_for_job_complete_with(uart_stream, tracker, timeout, BUSY_HEALTH_POLL_INTERVAL)
}

fn wait_for_job_complete_with<R: Read>(
    uart_stream: &mut UartLineStream<R>,
    tracker: &mut StatusTracker,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<bool> {
    tracker.set_state("BUSY");
    let busy_start = Instant::now();
    let mut last_health_poll = Instant::now();
    loop {
        if busy_start.elapsed() >= timeout {
            eprintln!("Timed out waiting for FPGA to complete job!");
            tracker.set_error("JOB_TIMEOUT", "Timed out waiting for FPGA to complete job");
            return Ok(false);
        }
        match uart_stream.next_event()? {
            UartEvent::Line(line) => {
                println!("UART: {}", line);
                if line.contains(ACTION_COMPLETE_SENTINEL) {
                    return Ok(true);
                }
                if line.contains(COMMAND_INPUT_SENTINEL) {
                    eprintln!("FPGA rebooted mid-job!");
                    tracker.set_error("JOB_REBOOTED", "FPGA rebooted mid-job");
                    return Ok(false);
                }
                if let Err(e) = active_runner_error_checks(&line) {
                    eprintln!("Runner/kernel error during job: {e}");
                    tracker.set_error("RUNNER_ERROR", e.to_string());
                    return Ok(false);
                }
            }
            UartEvent::TickTimeout => {}
            UartEvent::Eof => {
                return Err(anyhow!("unexpected EOF from FPGA UART during job"));
            }
        }

        if last_health_poll.elapsed() >= poll_interval {
            if let Some(info) = tracker.fpga_info().cloned() {
                last_health_poll = Instant::now();
                poll_health_tick(tracker, &info, "BUSY", false);
                if tracker.consecutive_health_failures() >= 3 {
                    eprintln!("FPGA failed 3 consecutive BUSY health checks; resetting");
                    tracker.set_state("UNHEALTHY");
                    return Ok(false);
                }
            }
        }
    }
}

fn get_sd_mux(
    sdwire: Option<&OsString>,
    usbsdmux: Option<&OsString>,
) -> anyhow::Result<Box<dyn SdMux>> {
    match (sdwire, usbsdmux) {
        (Some(sdwire), None) => {
            Ok(Box::new(SDWire::open(String::from(sdwire.to_str().unwrap()))?) as Box<dyn SdMux>)
        }
        (None, Some(usbsdmux)) => Ok(Box::new(UsbsdMux::open(String::from(
            usbsdmux.to_str().unwrap(),
        ))?) as Box<dyn SdMux>),
        _ => Err(anyhow!("One of --sdwire or --usbsdmux required")),
    }
}

fn main_impl() -> anyhow::Result<()> {
    let matches = cli().get_matches();
    let sdwire = matches.get_one::<OsString>("sdwire");
    let usbsdmux = matches.get_one::<OsString>("usbsdmux");

    let zcu104_path = matches.get_one::<UsbPortPath>("zcu104");
    let boss_ftdi_path = matches.get_one::<UsbPortPath>("boss_ftdi");

    let get_nonblocking_uart = || match (zcu104_path, boss_ftdi_path) {
        (Some(zcu104_path), None) => {
            ftdi_uart::open(zcu104_path.clone(), ftdi_interface::INTERFACE_B)
        }
        (None, Some(boss_ftdi_path)) => {
            ftdi_uart::open(boss_ftdi_path.clone(), ftdi_interface::INTERFACE_A)
        }
        _ => Err(anyhow!("One of --zcu104_path or --boss_ftdi_path required")),
    };

    let get_zcu104_path = || {
        zcu104_path
            .ok_or(anyhow!("--zcu104 flag required"))
            .cloned()
    };

    let get_fpga_ftdi = || FpgaJtag::open(get_zcu104_path()?);
    match matches.subcommand() {
        Some(("health", sub_matches)) => {
            let ip = sub_matches.get_one::<String>("IP").unwrap();
            let port = *sub_matches.get_one::<u16>("port").unwrap();
            let json_out = sub_matches.is_present("json");
            health::run_health_command(ip, port, json_out)?;
        }
        Some(("status", sub_matches)) => {
            let live = sub_matches.is_present("live");
            let json_out = sub_matches.is_present("json");
            health::run_status_command(live, json_out)?;
        }
        Some(("status-server", sub_matches)) => {
            let listen = sub_matches.get_one::<String>("listen").unwrap();
            health::run_status_server(listen)?;
        }
        Some(("mode", sub_matches)) => {
            let mut sd_mux = get_sd_mux(sdwire, usbsdmux)?;
            let mut fpga = get_fpga_ftdi();
            match sub_matches.get_one::<SdMuxTarget>("MODE").unwrap() {
                SdMuxTarget::Dut => {
                    if let Ok(fpga) = &mut fpga {
                        fpga.set_reset(FpgaReset::Reset)?;
                    }
                    sd_mux.set_target(SdMuxTarget::Dut)?;
                    std::thread::sleep(Duration::from_millis(1));
                    if let Ok(fpga) = &mut fpga {
                        fpga.set_reset(FpgaReset::Run)?;
                    }
                }
                SdMuxTarget::Host => {
                    if let Ok(fpga) = &mut fpga {
                        fpga.set_reset(FpgaReset::Reset)?;
                    }
                    sd_mux.set_target(SdMuxTarget::Host)?;
                }
            }
        }
        Some(("reset_fpga", _)) => {
            let mut fpga = get_fpga_ftdi();
            if let Ok(fpga) = &mut fpga {
                fpga.set_reset(FpgaReset::Reset)?;
                std::thread::sleep(Duration::from_millis(1));
                fpga.set_reset(FpgaReset::Run)?;
            } else {
                return Err(anyhow!("Error: failed to connect to FTDI chip."));
            }
        }
        Some(("reset_ftdi", _)) => {
            let mut fpga = get_fpga_ftdi()?;
            fpga.ftdi.reset()?;
        }
        Some(("console", _)) => {
            let (mut uart_rx, mut uart_tx) = get_nonblocking_uart()?;

            let mut stdout_buf = [0_u8; 4096];
            let mut stdin_buf = [0_u8; 4096];

            println!("To exit terminal type Ctrl-T then Q");
            // Will reset terminal back to regular settings upon drop
            let raw_terminal = tty::RawTerminal::from_stdin()?;

            let mut escaper = tty::EscapeSeqStateMachine::new();
            let mut alive = true;

            while alive {
                let mut stdin_len = tty::read_stdin_nonblock(&mut stdin_buf)?;
                escaper.process(&mut stdin_buf, &mut stdin_len, |ch| match ch {
                    b'q' | b'Q' => alive = false,
                    b'b' | b'B' => {
                        uart_tx.send_break().unwrap();
                    }
                    ch => println!(
                        "Unknown escape sequence: Ctrl-T + {:?}",
                        char::from_u32(ch.into()).unwrap_or('?')
                    ),
                });
                if stdin_len > 0 {
                    uart_tx.write_all(&stdin_buf[..stdin_len])?;
                }
                let stdout_len = uart_rx.read(&mut stdout_buf)?;
                if stdout_len > 0 {
                    stdout().write_all(&stdout_buf[..stdout_len])?;
                    stdout().flush()?;
                }
            }
            drop(raw_terminal);
            println!();
        }
        Some(("flash", sub_matches)) => {
            let mut sd_mux = get_sd_mux(sdwire, usbsdmux)?;
            let mut fpga = get_fpga_ftdi();
            let sd_dev_path = sd_mux.get_sd_dev_path()?;
            if let Ok(fpga) = &mut fpga {
                fpga.set_reset(FpgaReset::Reset)?;
            }
            sd_mux.set_target(SdMuxTarget::Host)?;
            let image_filename = sub_matches.get_one::<PathBuf>("IMAGE_FILENAME").unwrap();
            println!(
                "Flashing {} to {}",
                image_filename.display(),
                sd_dev_path.display()
            );
            let mut file =
                File::open(image_filename).with_context(|| image_filename.display().to_string())?;
            let mut sd_dev =
                open_block_dev(&sd_dev_path).with_context(|| sd_dev_path.display().to_string())?;
            if !sub_matches.is_present("read_first") || !verify_image(&mut sd_dev, &mut file)? {
                copy_file(&mut sd_dev, &mut file)?;
                if !verify_image(&mut sd_dev, &mut file)? {
                    return Err(anyhow!("Failed to verify image after flashing. Please try again."));
                }
            } else {
                println!("Device already contains the desired image");
            }
            sd_mux.set_target(SdMuxTarget::Dut)?;
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(fpga) = &mut fpga {
                fpga.set_reset(FpgaReset::Run)?
            }
        }

        Some(("serve", sub_matches)) => {
            let mut sd_mux = get_sd_mux(sdwire, usbsdmux)?;
            // Reuse the previous token if we never connect to GitHub.
            // This avoids creating a bunch of offline runners if the FPGA fails to establish a
            // connection.
            let mut cached_token: Option<Vec<u8>> = None;
            let mut fpga = get_fpga_ftdi()?;
            let sd_dev_path = sd_mux.get_sd_dev_path()?;

            let identifier =
                std::env::var("IDENTIFIER").unwrap_or_else(|_| "unknown".to_string());
            let mut tracker = StatusTracker::new(identifier);

            'outer: loop {
                if let Some(home_dir) = std::env::var_os("HOME") {
                    let stop_file = Path::new(&home_dir).join(".fpga-boss-stop");
                    if stop_file.exists() {
                        println!("Stop file {:?} found, exiting gracefully", stop_file);
                        tracker.set_state("STOPPED");
                        tracker.mark_stopped_cleanly();
                        break 'outer;
                    }
                }

                tracker.start_new_cycle();
                println!("Putting FPGA into reset");
                tracker.set_state("RESETTING");
                fpga.set_reset(FpgaReset::Reset)?;
                sd_mux.set_target(SdMuxTarget::Host)?;

                {
                    println!("Ensuring SD card contents are as expected");
                    let image_filename = sub_matches.get_one::<PathBuf>("IMAGE_FILENAME").unwrap();
                    let mut file = File::open(image_filename)
                        .with_context(|| image_filename.display().to_string())?;
                    let mut sd_dev = open_block_dev(&sd_dev_path)
                        .with_context(|| sd_dev_path.display().to_string())?;

                    if !verify_image(&mut sd_dev, &mut file)? {
                        copy_file(&mut sd_dev, &mut file)?;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }

                let (uart_rx, mut uart_tx) = ftdi_uart::open_blocking(
                    get_zcu104_path()?,
                    ftdi_interface::INTERFACE_B,
                    Some(UART_TICK_TIMEOUT),
                )?;
                let mut uart_stream = UartLineStream::new(uart_rx);

                println!("Taking FPGA out of reset");
                tracker.set_state("BOOTING");
                sd_mux.set_target(SdMuxTarget::Dut)?;
                std::thread::sleep(Duration::from_millis(100));
                fpga.set_reset(FpgaReset::Run)?;

                if !wait_for_boot(&mut uart_stream, &mut tracker, BOOT_TIMEOUT)? {
                    continue 'outer;
                }

                if !verify_pre_job_health(&mut tracker) {
                    continue 'outer;
                }

                if cached_token.is_none() {
                    let command_args: Vec<_> =
                        sub_matches.get_many::<OsString>("CMD").unwrap().collect();

                    println!("Executing command {:?} to retrieve jitconfig", command_args);
                    tracker.set_state("WAITING_FOR_JOB");
                    let output = std::process::Command::new(command_args[0])
                        .args(&command_args[1..])
                        .stderr(Stdio::inherit())
                        .output()?;
                    if !output.status.success() {
                        println!("Error retrieving jitconfig: stdout:");
                        stdout().write_all(&output.stdout)?;
                        tracker.set_error("JIT_CONFIG_ERROR", "jitconfig command failed");
                        continue 'outer;
                    }
                    cached_token = Some(output.stdout);
                }

                tracker.set_state("REGISTERING");
                uart_tx.write_all(b"runner-jitconfig ")?;
                uart_tx.write_all(
                    &cached_token
                        .clone()
                        .expect("A token should always be cached before sending it over the UART"),
                )?;
                uart_tx.write_all(b"\n")?;

                if !wait_for_registration(&mut uart_stream, &mut tracker, BOOT_TIMEOUT)? {
                    continue 'outer;
                }

                cached_token = None;

                if !wait_for_job_start(&mut uart_stream, &mut tracker)? {
                    continue 'outer;
                }

                if !wait_for_job_complete(&mut uart_stream, &mut tracker, JOB_TIMEOUT)? {
                    continue 'outer;
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    struct SliceStream(VecDeque<u8>);

    impl Read for SliceStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut n = 0;
            while n < buf.len() {
                let Some(b) = self.0.pop_front() else {
                    break;
                };
                buf[n] = b;
                n += 1;
            }
            Ok(n)
        }
    }

    #[test]
    fn test_boot_and_pre_job_health_with_mock_healthd() {
        let dir = TempDir::new().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let body = serde_json::json!({
            "healthy": true,
            "reasons": [],
            "hostname": "caliptra-fpga",
            "mode": "ci",
            "variant": "core",
            "build_commit": "deadbeef",
            "kernel_tainted": 0,
            "net_end0_carrier": 1,
            "uio_devices": ["uio0"],
            "runner_listener_alive": false,
            "runner_busy": false
        })
        .to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        });

        let uart_log = format!(
            "Booting Linux...\nCALIPTRA_FPGA_INFO {{\"ip\":\"{}\",\"health_port\":{},\"variant\":\"core\"}}\n{}\n",
            addr.ip(),
            addr.port(),
            COMMAND_INPUT_SENTINEL
        );
        let mut uart_stream = UartLineStream::new(SliceStream(uart_log.into_bytes().into()));
        let mut tracker =
            StatusTracker::with_state_dir("test-board", Some(dir.path().to_path_buf()));

        assert!(wait_for_boot(&mut uart_stream, &mut tracker, Duration::from_secs(5)).unwrap());
        assert!(verify_pre_job_health(&mut tracker));
        handle.join().unwrap();

        assert_eq!(tracker.record().http_status, Some(200));
        assert_eq!(
            tracker.record().health.as_ref().unwrap().build_commit,
            "deadbeef"
        );
        tracker.mark_stopped_cleanly();
    }

    #[test]
    fn test_boot_empty_ip_marks_dhcp_failed() {
        let dir = TempDir::new().unwrap();
        let uart_log = format!(
            "CALIPTRA_FPGA_INFO {{\"ip\":\"\",\"health_port\":8080,\"variant\":\"zcu104\"}}\n{}\n",
            COMMAND_INPUT_SENTINEL
        );
        let mut uart_stream = UartLineStream::new(SliceStream(uart_log.into_bytes().into()));
        let mut tracker =
            StatusTracker::with_state_dir("test-zcu", Some(dir.path().to_path_buf()));

        assert!(!wait_for_boot(&mut uart_stream, &mut tracker, Duration::from_secs(5)).unwrap());
        assert_eq!(tracker.record().state, "DHCP_FAILED");
        tracker.mark_stopped_cleanly();
    }

    #[test]
    fn test_busy_unexpected_reboot_fails_fast() {
        let dir = TempDir::new().unwrap();
        let uart_log = format!("some job line\n{}\n", COMMAND_INPUT_SENTINEL);
        let mut uart_stream = UartLineStream::new(SliceStream(uart_log.into_bytes().into()));
        let mut tracker =
            StatusTracker::with_state_dir("test-vck", Some(dir.path().to_path_buf()));

        assert!(
            !wait_for_job_complete(&mut uart_stream, &mut tracker, Duration::from_secs(60)).unwrap()
        );
        assert_eq!(tracker.record().state, "JOB_REBOOTED");
        tracker.mark_stopped_cleanly();
    }

    enum TickStep {
        Line(&'static [u8]),
        Timeout,
    }

    struct TickReader(VecDeque<TickStep>);

    impl Read for TickReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.pop_front() {
                Some(TickStep::Line(bytes)) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    Ok(n)
                }
                Some(TickStep::Timeout) => Err(Error::new(ErrorKind::TimedOut, "tick")),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn test_idle_and_busy_consecutive_health_failures_trigger_reset() {
        let dir = TempDir::new().unwrap();

        // IDLE: 2 consecutive polls with runner_listener_alive=false (after grace=0) -> UNHEALTHY
        let idle_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let idle_addr = idle_listener.local_addr().unwrap();
        let idle_body = serde_json::json!({
            "healthy": true,
            "reasons": [],
            "hostname": "caliptra-fpga",
            "mode": "ci",
            "variant": "subsystem",
            "build_commit": "abc",
            "runner_listener_alive": false,
            "runner_busy": false
        })
        .to_string();
        let idle_handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = idle_listener.accept().unwrap();
                let mut buf = [0_u8; 512];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    idle_body.len(),
                    idle_body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });

        let mut tracker =
            StatusTracker::with_state_dir("test-idle", Some(dir.path().to_path_buf()));
        tracker.set_fpga_info(FpgaInfo {
            ip: idle_addr.ip().to_string(),
            health_port: idle_addr.port(),
            variant: "subsystem".to_string(),
        });
        let mut idle_stream = UartLineStream::new(TickReader(VecDeque::from([
            TickStep::Timeout,
            TickStep::Timeout,
        ])));
        assert!(
            !wait_for_job_start_with(
                &mut idle_stream,
                &mut tracker,
                Duration::ZERO,
                Duration::ZERO,
            )
            .unwrap()
        );
        idle_handle.join().unwrap();
        assert_eq!(tracker.record().state, "UNHEALTHY");
        assert_eq!(tracker.record().consecutive_health_failures, 2);
        assert_eq!(
            tracker.record().last_error.as_deref(),
            Some("IDLE runner_listener_not_alive")
        );
        tracker.mark_stopped_cleanly();

        // BUSY: 3 consecutive 503 polls -> UNHEALTHY
        let busy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy_addr = busy_listener.local_addr().unwrap();
        let busy_body = serde_json::json!({
            "healthy": false,
            "reasons": ["low_memory"],
            "hostname": "caliptra-fpga",
            "mode": "ci",
            "variant": "subsystem",
            "build_commit": "abc",
            "runner_listener_alive": true,
            "runner_busy": true
        })
        .to_string();
        let busy_handle = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = busy_listener.accept().unwrap();
                let mut buf = [0_u8; 512];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    busy_body.len(),
                    busy_body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });

        let mut busy_tracker =
            StatusTracker::with_state_dir("test-busy", Some(dir.path().to_path_buf()));
        busy_tracker.set_fpga_info(FpgaInfo {
            ip: busy_addr.ip().to_string(),
            health_port: busy_addr.port(),
            variant: "subsystem".to_string(),
        });
        let mut busy_stream = UartLineStream::new(TickReader(VecDeque::from([
            TickStep::Line(b"running test\n"),
            TickStep::Timeout,
            TickStep::Timeout,
        ])));
        assert!(
            !wait_for_job_complete_with(
                &mut busy_stream,
                &mut busy_tracker,
                Duration::from_secs(60),
                Duration::ZERO,
            )
            .unwrap()
        );
        busy_handle.join().unwrap();
        assert_eq!(busy_tracker.record().state, "UNHEALTHY");
        assert_eq!(busy_tracker.record().consecutive_health_failures, 3);
        busy_tracker.mark_stopped_cleanly();
    }
}
