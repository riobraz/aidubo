//! PTY management and the WebSocket byte stream.
//!
//! The contract was measured against a live server; the evidence and the
//! transport decision are in `docs/spikes/pty-transport.md` and ADR 0003.

use std::collections::HashMap;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures_util::{SinkExt as _, StreamExt as _};
use harness_core::TerminalBatch;
use harness_protocol::{CreatePtyRequest, Envelope, PtyProcess, PtyTicket};
use reqwest::Method;
use serde::Deserialize;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};
use tokio_tungstenite::tungstenite::Message;

use crate::EngineEvent;
use crate::api::{SessionApi, decode_error, ensure_success, network_error};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TICKET_HEADER: &str = "x-opencode-ticket";
const NOT_CONNECTED: &str = "The terminal is not connected.";

impl SessionApi {
    pub async fn create_pty(
        &self,
        directory: &str,
        request: CreatePtyRequest,
    ) -> Result<PtyProcess, String> {
        let response = self
            .request(Method::POST, "/api/pty")
            .query(&[("location[directory]", directory)])
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<PtyProcess> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn ptys(&self, directory: &str) -> Result<Vec<PtyProcess>, String> {
        let response = self
            .request(Method::GET, "/api/pty")
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<PtyProcess>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn resize_pty(
        &self,
        directory: &str,
        id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<PtyProcess, String> {
        let response = self
            .request(Method::PUT, &format!("/api/pty/{id}"))
            .query(&[("location[directory]", directory)])
            .json(&serde_json::json!({ "size": { "rows": rows, "cols": cols } }))
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<PtyProcess> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn delete_pty(&self, directory: &str, id: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/pty/{id}"))
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }

    /// Tickets are single-use and expire after about a minute; request one
    /// immediately before opening the socket.
    pub async fn pty_ticket(&self, directory: &str, id: &str) -> Result<PtyTicket, String> {
        let response = self
            .request(Method::POST, &format!("/api/pty/{id}/connect-token"))
            .query(&[("location[directory]", directory)])
            .header(TICKET_HEADER, "1")
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<PtyTicket> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    /// Build the socket URL. The ticket is a query parameter and is never
    /// logged or included in an error message.
    pub(crate) fn pty_connect_url(
        &self,
        directory: &str,
        id: &str,
        ticket: &str,
        cursor: Option<u64>,
    ) -> Result<String, String> {
        let base = self.base_url.replacen("http://", "ws://", 1);
        let mut url = reqwest::Url::parse(&format!("{base}/api/pty/{id}/connect"))
            .map_err(|_| "Could not build the terminal connection URL.".to_string())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("ticket", ticket);
            if let Some(cursor) = cursor {
                query.append_pair("cursor", &cursor.to_string());
            }
            query.append_pair("location[directory]", directory);
        }
        Ok(url.to_string())
    }
}

/// One open socket plus the channel that writes input to it.
pub(crate) struct PtyLink {
    input: Sender<Vec<u8>>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct PtyLinks {
    links: HashMap<String, PtyLink>,
}

impl PtyLinks {
    pub(crate) fn is_connected(&self, id: &str) -> bool {
        self.links
            .get(id)
            .is_some_and(|link| !link.input.is_closed())
    }

    pub(crate) fn insert(&mut self, id: String, input: Sender<Vec<u8>>, task: JoinHandle<()>) {
        if let Some(previous) = self.links.insert(id, PtyLink { input, task }) {
            previous.task.abort();
        }
    }

    /// Drop the socket writer and stop the reader task.
    pub(crate) fn close(&mut self, id: &str) {
        if let Some(link) = self.links.remove(id) {
            link.task.abort();
        }
    }

    /// Queue raw input for the socket; never blocks the engine loop.
    pub(crate) fn write(&self, id: &str, bytes: Vec<u8>) -> Result<(), String> {
        let link = self
            .links
            .get(id)
            .ok_or_else(|| NOT_CONNECTED.to_string())?;
        link.input
            .try_send(bytes)
            .map_err(|_| "The terminal input queue is closed.".to_string())
    }
}

#[derive(Deserialize)]
struct CursorFrame {
    cursor: u64,
}

/// The server reports its byte cursor in a binary frame prefixed with NUL.
fn parse_cursor(bytes: &[u8]) -> Option<u64> {
    let json = bytes.strip_prefix(&[0])?;
    serde_json::from_slice::<CursorFrame>(json)
        .ok()
        .map(|frame| frame.cursor)
}

async fn send_output(event_tx: &Sender<EngineEvent>, id: &str, bytes: Vec<u8>) {
    if bytes.is_empty() {
        return;
    }
    let _ = event_tx
        .send(EngineEvent::PtyOutput {
            id: id.to_string(),
            bytes,
        })
        .await;
}

/// Open the socket and forward output until it closes.
///
/// The URL carries a ticket, so connection failures are reported with a fixed
/// message instead of forwarding an error that could echo it.
pub(crate) async fn pump_pty(
    id: String,
    url: String,
    input: Receiver<Vec<u8>>,
    event_tx: Sender<EngineEvent>,
) {
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url))
        .await
        .ok()
        .and_then(Result::ok);
    let Some((socket, _response)) = connected else {
        let _ = event_tx
            .send(EngineEvent::PtyError {
                id,
                message: "Could not open the terminal connection.".into(),
            })
            .await;
        return;
    };

    let (mut writer, mut reader) = socket.split();
    let mut batch = TerminalBatch::default();
    let mut deadline: Option<Instant> = None;
    let mut error: Option<&str> = None;

    loop {
        let wait_at = deadline;
        let timer = async {
            match wait_at {
                Some(at) => sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            message = reader.next() => {
                let Some(message) = message else { break };
                match message {
                    Ok(Message::Text(text)) => {
                        let chunks = batch.push(text.as_bytes(), Instant::now().into());
                        for chunk in chunks {
                            send_output(&event_tx, &id, chunk).await;
                        }
                        deadline = batch.deadline().map(Instant::from_std);
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Some(cursor) = parse_cursor(&bytes) {
                            tracing::debug!(pty = %id, cursor, "terminal cursor");
                        } else {
                            let chunks = batch.push(&bytes, Instant::now().into());
                            for chunk in chunks {
                                send_output(&event_tx, &id, chunk).await;
                            }
                            deadline = batch.deadline().map(Instant::from_std);
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                    Err(_) => {
                        error = Some("The terminal connection failed.");
                        break;
                    }
                }
            }
            bytes = input.recv() => {
                let Ok(bytes) = bytes else { break };
                if writer.send(Message::binary(bytes)).await.is_err() {
                    error = Some("The terminal connection failed.");
                    break;
                }
            }
            _ = timer => {
                send_output(&event_tx, &id, batch.take()).await;
                deadline = None;
            }
        }
    }

    send_output(&event_tx, &id, batch.take()).await;
    let message = match error {
        Some(message) => EngineEvent::PtyError {
            id,
            message: message.to_string(),
        },
        None => EngineEvent::PtyClosed { id },
    };
    let _ = event_tx.send(message).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_cursor_control_is_not_output() {
        assert_eq!(parse_cursor(b"\x00{\"cursor\":5}"), Some(5));
        assert_eq!(parse_cursor(b"hello"), None);
        assert_eq!(parse_cursor(b"\x00not json"), None);
    }
}
