//! The firewall switch on the Mobile page. Phones reach the desktop on UDP
//! [`PORT`], and a firewall that drops it leaves them with "Cannot reach your
//! desktop" even on the same Wi-Fi. Changing a rule needs administrator
//! rights, so the OS asks for them on every switch.
use monocode_mobile_link::PORT;

/// The firewall the switch changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Windows,
    Firewalld,
    Ufw,
}

/// The firewall on this desktop and whether it lets phones in. The state is
/// `None` for ufw, which shows its rules only to root. `None` overall when
/// no firewall the switch knows is running.
pub fn check() -> Option<(Kind, Option<bool>)> {
    #[cfg(windows)]
    {
        let open = windows::open(&std::env::current_exe().ok()?).ok()?;
        Some((Kind::Windows, Some(open)))
    }
    #[cfg(target_os = "linux")]
    {
        linux::check()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// Lets phones in, or shuts them out again. Fails when the change is refused
/// or its permission prompt is dismissed.
pub fn set(kind: Kind, on: bool) -> Result<(), String> {
    #[cfg(windows)]
    let done = kind == Kind::Windows && windows::set(on);
    #[cfg(target_os = "linux")]
    let done = linux::set(kind, on);
    #[cfg(not(any(windows, target_os = "linux")))]
    let done = {
        let _ = (kind, on);
        false
    };
    done.then_some(())
        .ok_or_else(|| "The firewall wasn't changed.".into())
}

#[cfg(windows)]
mod windows {
    use super::PORT;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::{path::Path, process::Command};

    const INBOUND: &str = "Get-NetFirewallApplicationFilter | Where-Object { $_.Program -eq $exe } | Get-NetFirewallRule | Where-Object Direction -eq 'Inbound'";

    /// Whether `exe` has an enabled inbound allow rule for UDP and no block
    /// rule for it. Reading rules needs no administrator rights.
    pub(super) fn open(exe: &Path) -> Result<bool, String> {
        let output = powershell(&format!(
            "{}\n$rules = @({INBOUND} | Where-Object {{ $_.Enabled -eq 'True' -and @('UDP', 'Any') -contains ($_ | Get-NetFirewallPortFilter).Protocol }})\n\
             if (($rules | Where-Object Action -eq 'Allow') -and -not ($rules | Where-Object Action -eq 'Block')) {{ 'open' }} else {{ 'closed' }}",
            program(exe)
        ))
        .output()
        .map_err(|e| e.to_string())?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "open" => Ok(true),
            "closed" => Ok(false),
            _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        }
    }

    /// Drops this program's inbound rules, among them the block rules that a
    /// cancelled "Allow access" prompt leaves, then adds the allow rule when
    /// `on`. Runs behind one UAC prompt; false when it is declined.
    pub(super) fn set(on: bool) -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let mut change = format!(
            "$ErrorActionPreference = 'Stop'\n{}\n{INBOUND} | Remove-NetFirewallRule",
            program(&exe)
        );
        if on {
            change += &format!(
                "\nNew-NetFirewallRule -DisplayName 'MCode mobile link' -Direction Inbound -Action Allow -Program $exe -Protocol UDP -LocalPort {PORT} -Profile Private,Public | Out-Null"
            );
        }
        powershell(&format!(
            "$ErrorActionPreference = 'Stop'\n$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{}'\nexit $p.ExitCode",
            encode(&change)
        ))
        .status()
        .is_ok_and(|status| status.success())
    }

    /// `$exe = …` with the path in base64, so no character in it can end
    /// the string.
    fn program(exe: &Path) -> String {
        format!(
            "$exe = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))",
            STANDARD.encode(exe.to_string_lossy().as_bytes())
        )
    }

    /// PowerShell's `-EncodedCommand` form: base64 of UTF-16LE.
    fn encode(script: &str) -> String {
        STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        )
    }

    fn powershell(script: &str) -> Command {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encode(script),
        ]);
        crate::sys::process::configure_child(&mut command);
        command
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Kind, PORT};
    use std::process::{Command, Stdio};

    pub(super) fn check() -> Option<(Kind, Option<bool>)> {
        if succeeds("firewall-cmd", &["--state"]) {
            let open = succeeds("firewall-cmd", &[&format!("--query-port={PORT}/udp")]);
            return Some((Kind::Firewalld, Some(open)));
        }
        std::fs::read_to_string("/etc/ufw/ufw.conf")
            .ok()?
            .lines()
            .any(|line| line.trim() == "ENABLED=yes")
            .then_some((Kind::Ufw, None))
    }

    /// Runs the change as root behind polkit's password prompt.
    // ponytail: firewalld's default zone only; a network bound to another
    // zone needs `--zone`.
    pub(super) fn set(kind: Kind, on: bool) -> bool {
        let script = match (kind, on) {
            (Kind::Firewalld, true) => format!(
                "firewall-cmd --add-port={PORT}/udp && firewall-cmd --permanent --add-port={PORT}/udp"
            ),
            (Kind::Firewalld, false) => format!(
                "firewall-cmd --remove-port={PORT}/udp && firewall-cmd --permanent --remove-port={PORT}/udp"
            ),
            (Kind::Ufw, true) => format!("ufw allow {PORT}/udp"),
            (Kind::Ufw, false) => format!("ufw delete allow {PORT}/udp"),
            (Kind::Windows, _) => return false,
        };
        succeeds("pkexec", &["/bin/sh", "-c", &script])
    }

    fn succeeds(program: &str, args: &[&str]) -> bool {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::sys::process::configure_child(&mut command);
        command.status().is_ok_and(|status| status.success())
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn a_path_with_quotes_is_read_without_ending_the_script_early() {
        let exe = std::path::Path::new(r"C:\no such dir\it's ‘odd’ $(x).exe");
        assert_eq!(super::windows::open(exe), Ok(false));
    }
}
