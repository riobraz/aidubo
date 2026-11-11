//! Channels between the workspace and the app-owned notification backend.
//!
//! The view decides *whether* to notify; the app owns the OS backend and the
//! settings file. Both directions are plain channels, so no disk or process
//! work happens on the GPUI thread.

use async_channel::{Receiver, Sender};
use harness_core::{Notification, NotifySettings};

/// A delivery problem reported back by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyStatus {
    /// The OS refused (or silently dropped) a notification.
    Denied,
}

#[derive(Clone)]
pub struct NotifyChannels {
    pub notifications: Sender<Notification>,
    pub settings: Sender<NotifySettings>,
    pub status: Receiver<NotifyStatus>,
}

impl NotifyChannels {
    /// A detached set for tests: sends succeed into an unread queue and no
    /// delivery error is ever reported.
    pub fn disabled() -> Self {
        let (notifications, _receiver) = async_channel::unbounded();
        let (settings, _settings_receiver) = async_channel::unbounded();
        let (_status_sender, status) = async_channel::unbounded();
        Self {
            notifications,
            settings,
            status,
        }
    }
}
