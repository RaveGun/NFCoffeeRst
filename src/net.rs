//! Device connectivity: WiFi association helpers and the TCP data transfer.

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Local};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::protocol::{build_secret_key, parse_transfer_line};

/// Callback used to stream human-readable progress to the UI.
pub type LogFn<'a> = &'a mut dyn FnMut(String);

/// Full transfer: ensure connectivity, handshake, read all records.
///
/// Connection failures are retried (the device may not have finished starting
/// its AP / server yet). A *successful* TCP connect means the device is ready,
/// so we must complete the handshake immediately -- see `prepare_connection`.
pub fn run_transfer(cfg: &Config, log: LogFn) -> Result<Vec<(String, u32)>> {
    log(format!("Target device {}:{}", cfg.host, cfg.port));
    prepare_connection(cfg, log);

    let key = current_key(cfg);
    log(format!("Using date key for {}.", current_date_string(cfg)));

    let deadline = Instant::now() + Duration::from_secs(cfg.connect_timeout_secs.max(1));
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match try_fetch(cfg, &key, log) {
            Ok(records) => return Ok(records),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(e).context(format!(
                        "device not reachable after {} attempt(s); is the master card \
                         presented and are you connected to '{}'?",
                        attempt, cfg.ssid
                    ));
                }
                log(format!("Attempt {attempt} failed: {e}. Retrying..."));
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// One TCP connect + handshake + read. Never results in a bare probe: if the
/// connect succeeds we always send the key immediately.
fn try_fetch(cfg: &Config, key: &[u8; 8], log: LogFn) -> Result<Vec<(String, u32)>> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let sock = addr
        .to_socket_addrs()
        .with_context(|| format!("resolving {addr}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("no address for {addr}"))?;

    let mut stream = TcpStream::connect_timeout(&sock, Duration::from_secs(3))
        .with_context(|| format!("connecting to {addr}"))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(
        cfg.transfer_timeout_secs.max(1),
    )))?;

    stream.write_all(key)?;
    stream.flush()?;
    log("Handshake key sent; receiving data...".to_string());

    let reader = BufReader::new(stream);
    let mut records: Vec<(String, u32)> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(l) => {
                if let Some(rec) = parse_transfer_line(&l) {
                    records.push((rec.codename, rec.count));
                }
            }
            // A read timeout means the device is done sending.
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break
            }
            Err(e) => return Err(e).context("reading transfer stream"),
        }
    }

    log(format!("Received {} record(s).", records.len()));
    Ok(records)
}

fn current_key(cfg: &Config) -> [u8; 8] {
    let now = Local::now();
    let month1 = now.month() as u8; // chrono is 1-based
    let month = if cfg.zero_based_month { month1 - 1 } else { month1 };
    build_secret_key(now.year() as u16, month, now.day() as u8)
}

fn current_date_string(cfg: &Config) -> String {
    let now = Local::now();
    let month1 = now.month() as u8;
    let month = if cfg.zero_based_month { month1 - 1 } else { month1 };
    format!("{:04}{:02}{:02}", now.year(), month, now.day())
}

/// Bring up connectivity before the real transfer attempt.
///
/// Deliberately does **not** probe the device TCP port: a bare connect/close
/// makes the firmware's `ReadSecretKey` time out and drops it out of transfer
/// mode. We only run the association step here; the actual connect happens in
/// `try_fetch` (which always completes the handshake).
fn prepare_connection(cfg: &Config, log: LogFn) {
    if !cfg.pre_connect_cmd.trim().is_empty() {
        log(format!("Running pre-connect command: {}", cfg.pre_connect_cmd.trim()));
        match run_shell(cfg.pre_connect_cmd.trim()) {
            Ok(out) => {
                for l in out.lines().filter(|l| !l.trim().is_empty()) {
                    log(format!("  {l}"));
                }
            }
            Err(e) => log(format!("  pre-connect command error: {e}")),
        }
        return;
    }

    if cfg.auto_connect {
        log(format!("Trying to join WiFi AP '{}' ...", cfg.ssid));
        if let Err(e) = platform_connect(cfg, log) {
            log(format!("Auto-connect failed: {e}"));
            log(format!(
                "Join the AP '{}' manually, then press Transfer again.",
                cfg.ssid
            ));
        }
    } else {
        log("Auto-connect disabled; assuming the device AP is already joined.".to_string());
    }
}

// --- platform specific association -----------------------------------------

#[cfg(target_os = "linux")]
fn platform_connect(cfg: &Config, log: LogFn) -> Result<()> {
    if !command_exists("nmcli") {
        bail!("nmcli (NetworkManager) not found");
    }
    // Non-destructive scan (does not touch the device TCP port).
    match run_capture("nmcli", &["-t", "-f", "SSID", "device", "wifi", "list"]) {
        Ok(out) => {
            if out.lines().any(|l| l.trim() == cfg.ssid) {
                log(format!("AP '{}' is visible.", cfg.ssid));
            } else {
                log(format!("AP '{}' not seen yet; trying to connect anyway.", cfg.ssid));
            }
        }
        Err(e) => log(format!("Scan failed: {e}")),
    }
    let out = run_capture(
        "nmcli",
        &[
            "--wait",
            "30",
            "device",
            "wifi",
            "connect",
            &cfg.ssid,
            "password",
            &cfg.password,
        ],
    )?;
    for l in out.lines().filter(|l| !l.trim().is_empty()) {
        log(format!("  {l}"));
    }
    Ok(())
}

#[cfg(windows)]
fn platform_connect(cfg: &Config, log: LogFn) -> Result<()> {
    if !command_exists("netsh") {
        bail!("netsh not found");
    }
    // Best effort: create/refresh a user profile so `connect` works even if the
    // profile does not exist yet. This does not require administrator rights.
    let xml = wlan_profile_xml(cfg);
    let tmp = std::env::temp_dir().join("nfcoffee_wlan.xml");
    if std::fs::write(&tmp, xml).is_ok() {
        let arg = format!("filename={}", tmp.display());
        match run_capture("netsh", &["wlan", "add", "profile", &arg, "user=current"]) {
            Ok(out) => log(format!("  {}", out.trim())),
            Err(e) => log(format!("  add profile: {e}")),
        }
    }
    let name = format!("name={}", cfg.ssid);
    let ssid = format!("ssid={}", cfg.ssid);
    let out = run_capture("netsh", &["wlan", "connect", &name, &ssid])?;
    log(format!("  {}", out.trim()));
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn platform_connect(_cfg: &Config, _log: LogFn) -> Result<()> {
    bail!("automatic WiFi connect is not implemented on this platform")
}

#[cfg(windows)]
fn wlan_profile_xml(cfg: &Config) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>{ssid}</name>
  <SSIDConfig>
    <SSID><name>{ssid}</name></SSID>
  </SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>manual</connectionMode>
  <MSM>
    <security>
      <authEncryption>
        <authentication>WPA2PSK</authentication>
        <encryption>AES</encryption>
        <useOneX>false</useX>
      </authEncryption>
      <sharedKey>
        <keyType>passPhrase</keyType>
        <protected>false</protected>
        <keyMaterial>{key}</keyMaterial>
      </sharedKey>
    </security>
  </MSM>
</WLANProfile>
"#,
        ssid = esc(&cfg.ssid),
        key = esc(&cfg.password),
    )
}

// --- process helpers --------------------------------------------------------

fn command_exists(prog: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(prog)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new(prog)
            .arg("--version")
            .output()
            .map(|_| true)
            .unwrap_or(false)
    }
}

fn run_capture(prog: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(prog)
        .args(args)
        .output()
        .with_context(|| format!("running {prog}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        if text.trim().is_empty() {
            text = format!("{prog} exited with {}", output.status);
        }
    }
    Ok(text)
}

#[cfg(windows)]
fn run_shell(cmd: &str) -> Result<String> {
    run_capture("cmd", &["/C", cmd])
}

#[cfg(not(windows))]
fn run_shell(cmd: &str) -> Result<String> {
    run_capture("sh", &["-c", cmd])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// Emulate the firmware: read the 8-byte key, validate it, then stream
    /// `CODENAME,count` lines and close.
    #[test]
    fn fetch_records_from_mock_device() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut key = [0u8; 8];
            sock.read_exact(&mut key).unwrap();

            // Same validation as WLAN::ReadSecretKey.
            let chk1 = ((key[0] as u16) << 8) | key[1] as u16;
            let chk2 = ((key[7] as u16) << 8) | key[6] as u16;
            let ymd = key[2] as u16 * 256 + key[3] as u16 + key[4] as u16 + key[5] as u16;
            assert_eq!(chk1, chk2, "checksums must match");
            assert_eq!(ymd, chk1, "year+month+day must equal checksum");

            sock.write_all(b"00001FY0.V1E,42\r\n").unwrap();
            sock.write_all(b"00000000.001,7\r\n").unwrap();
            sock.write_all(b"garbage line\n").unwrap();
        });

        let cfg = Config {
            host: addr.ip().to_string(),
            port: addr.port(),
            connect_timeout_secs: 5,
            ..Config::default()
        };
        let key = build_secret_key(2026, 9, 14);
        let mut lines = Vec::new();
        let mut log = |m: String| lines.push(m);
        let records = try_fetch(&cfg, &key, &mut log).unwrap();
        server.join().unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0], ("00001FY0.V1E".to_string(), 42));
        assert_eq!(records[1], ("00000000.001".to_string(), 7));
    }
}
