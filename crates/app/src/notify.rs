//! OS notification backends, settings persistence, and delivery threads.
//!
//! Everything here is IO; the view only decides whether a notification should
//! exist and hands it over a channel. The backends are deliberately tiny and
//! content-free in their error messages.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;

use async_channel::{Receiver, Sender};
use harness_core::{Notification, NotifySettings};
use harness_views::NotifyStatus;

pub const NOTIFY_SINK_ENV: &str = "RUST_HARNESS_NOTIFY_SINK";
pub const NOTIFY_SETTINGS_ENV: &str = "RUST_HARNESS_NOTIFY_SETTINGS";

pub trait Notifier: Send {
    fn notify(&self, notification: &Notification) -> Result<(), String>;
}

/// The script is constant; the title and body always travel as argv so no
/// message text can become AppleScript source.
#[cfg(target_os = "macos")]
const MAC_SCRIPT: &str =
    "on run argv\n    display notification (item 2 of argv) with title (item 1 of argv)\nend run";

#[cfg(target_os = "macos")]
pub struct MacNotifier;

#[cfg(target_os = "macos")]
impl Notifier for MacNotifier {
    fn notify(&self, notification: &Notification) -> Result<(), String> {
        let output = mac_command(notification)
            .output()
            .map_err(|_| "could not start the notification helper".to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err("the system did not show the notification".to_string())
        }
    }
}

#[cfg(target_os = "macos")]
fn mac_command(notification: &Notification) -> Command {
    let mut command = Command::new("/usr/bin/osascript");
    command
        .arg("-e")
        .arg(MAC_SCRIPT)
        .arg(&notification.session_title)
        .arg(&notification.body);
    command
}

#[cfg(target_os = "linux")]
pub struct LinuxNotifier;

#[cfg(target_os = "linux")]
impl Notifier for LinuxNotifier {
    fn notify(&self, notification: &Notification) -> Result<(), String> {
        let output = Command::new("notify-send")
            .arg(&notification.session_title)
            .arg(&notification.body)
            .output()
            .map_err(|_| "could not start notify-send".to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err("the system did not show the notification".to_string())
        }
    }
}

/// One JSON line per notification, for tests and `RUST_HARNESS_NOTIFY_SINK`.
pub struct FileNotifier {
    path: PathBuf,
}

impl FileNotifier {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Notifier for FileNotifier {
    fn notify(&self, notification: &Notification) -> Result<(), String> {
        let line = serde_json::to_string(notification)
            .map_err(|_| "could not encode the notification".to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|_| "could not open the notification sink".to_string())?;
        writeln!(file, "{line}").map_err(|_| "could not write the notification sink".to_string())
    }
}

pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn notify(&self, _notification: &Notification) -> Result<(), String> {
        Ok(())
    }
}

pub fn from_env() -> Box<dyn Notifier> {
    if let Some(path) = std::env::var_os(NOTIFY_SINK_ENV) {
        return Box::new(FileNotifier::new(PathBuf::from(path)));
    }

    #[cfg(target_os = "macos")]
    {
        Box::new(MacNotifier)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(LinuxNotifier)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Box::new(NullNotifier)
    }
}

/// Where settings live: the test override, the macOS app-support directory, or
/// the XDG config directory.
pub fn settings_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(NOTIFY_SETTINGS_ENV) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        home.map(|home| home.join("Library/Application Support/Rust Harness/notifications.json"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(config).join("rust-harness/notifications.json"));
        }
        home.map(|home| home.join(".config/rust-harness/notifications.json"))
    }
}

/// Missing or invalid files fall back to defaults; a bad file never prevents
/// the app from starting.
pub fn load_settings() -> NotifySettings {
    let Some(path) = settings_path() else {
        return NotifySettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| {
            tracing::warn!("ignoring an invalid notification settings file");
            NotifySettings::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => NotifySettings::default(),
        Err(_) => {
            tracing::warn!("could not read the notification settings file");
            NotifySettings::default()
        }
    }
}

pub fn save_settings(settings: &NotifySettings) -> Result<(), String> {
    let Some(path) = settings_path() else {
        return Err("no settings directory is available".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "could not create the settings directory".to_string())?;
    }
    let raw = serde_json::to_vec_pretty(settings)
        .map_err(|_| "could not encode the notification settings".to_string())?;
    std::fs::write(&path, raw).map_err(|_| "could not write the settings file".to_string())
}

/// One thread delivers notifications, one persists settings changes. Neither
/// touches the UI thread.
pub fn spawn_delivery(
    notifier: Box<dyn Notifier>,
    notifications: Receiver<Notification>,
    settings: Receiver<NotifySettings>,
    status: Sender<NotifyStatus>,
) {
    let delivery = std::thread::Builder::new()
        .name("notify-delivery".into())
        .spawn(move || {
            while let Ok(notification) = notifications.recv_blocking() {
                if notifier.notify(&notification).is_err() {
                    tracing::warn!("the OS did not accept a notification");
                    if status.send_blocking(NotifyStatus::Denied).is_err() {
                        break;
                    }
                }
            }
        });
    if delivery.is_err() {
        tracing::warn!("could not start the notification delivery thread");
    }

    let persistence = std::thread::Builder::new()
        .name("notify-settings".into())
        .spawn(move || {
            while let Ok(settings) = settings.recv_blocking() {
                if save_settings(&settings).is_err() {
                    tracing::warn!("could not persist the notification settings");
                }
            }
        });
    if persistence.is_err() {
        tracing::warn!("could not start the notification settings thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_core::NotifyKind;

    fn notification() -> Notification {
        Notification {
            id: "s1:turn_done:evt_1".into(),
            kind: NotifyKind::TurnDone,
            session_title: "Fix the build".into(),
            body: "Turn finished".into(),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn osascript_receives_body_as_argv() {
        let mut notification = notification();
        notification.body = "\"; do shell script \"touch /tmp/should-not-exist".into();
        let command = mac_command(&notification);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert!(args.iter().any(|arg| arg == MAC_SCRIPT));
        assert!(args.iter().any(|arg| arg == "Fix the build"));
        assert!(args.contains(&notification.body));
        // The script is constant and contains none of the payload.
        assert!(!MAC_SCRIPT.contains("Fix the build"));
        assert!(!MAC_SCRIPT.contains(&notification.body));
    }

    #[test]
    fn file_notifier_appends_json_line() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rust-harness-notify-{}-{unique}.jsonl",
            std::process::id()
        ));
        let notifier = FileNotifier::new(path.clone());

        notifier.notify(&notification()).expect("first line");
        notifier.notify(&notification()).expect("second line");

        let raw = std::fs::read_to_string(&path).expect("read sink");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("json line");
        assert_eq!(parsed["kind"], "turn_done");
        assert_eq!(parsed["session_title"], "Fix the build");
        assert_eq!(parsed["body"], "Turn finished");

        std::fs::remove_file(&path).expect("clean up sink");
    }
}
