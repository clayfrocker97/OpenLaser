// SPDX-License-Identifier: GPL-3.0-or-later

//! One UDP socket to the controller, one exchange at a time.
//!
//! An exchange sends a request once and waits for its reply. A reply from
//! the wrong peer or for another transaction is discarded and waiting
//! continues, up to the vendor's three receive attempts. Nothing is ever
//! re-sent within an exchange: a write without a reply is uncertain. The
//! controller task may start a fresh feedback poll after a read timeout;
//! those retries yield to urgent controls and never repeat a write.

use crate::config::Config;
use openlaser_protocol::records::timing;
use openlaser_protocol::requests::{Read, Write};
use openlaser_protocol::{Direction, Frame, FrameError, MAX_FRAME_BYTES};
use std::net::{SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio::net::UdpSocket;

/// Why an exchange failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// The socket could not be opened or used.
    #[error("socket: {0}")]
    Io(String),
    /// No acceptable reply arrived in time.
    #[error("no reply to {0} of register {1}")]
    NoReply(&'static str, u32),
    /// The reply was not a frame, or did not answer the request.
    #[error("bad reply: {0}")]
    Reply(FrameError),
}

/// The socket, the transaction counter and the receive buffer.
pub struct Link {
    socket: UdpSocket,
    peer: SocketAddrV4,
    timeout: Duration,
    transaction: u16,
    buffer: Vec<u8>,
}

impl Link {
    /// Opens a socket on the configured host address towards the endpoint.
    pub async fn open(config: &Config) -> Result<Self, LinkError> {
        let socket = UdpSocket::bind(SocketAddrV4::new(config.host, 0))
            .await
            .map_err(|e| LinkError::Io(e.to_string()))?;
        let seed = u16::try_from(std::process::id() & 0xffff).unwrap_or(1);
        Ok(Self {
            socket,
            peer: config.endpoint,
            timeout: config.reply_timeout,
            transaction: seed,
            buffer: vec![0; MAX_FRAME_BYTES],
        })
    }

    /// The controller's endpoint.
    #[must_use]
    pub const fn peer(&self) -> SocketAddrV4 {
        self.peer
    }

    /// Reads a block of words.
    pub async fn read(&mut self, read: Read) -> Result<Vec<u32>, LinkError> {
        let request = read.frame(self.next_transaction()).map_err(LinkError::Reply)?;
        let reply = self.exchange(request, "read").await?;
        Ok(reply.values)
    }

    /// Writes words. `Ok` means the controller acknowledged the write.
    pub async fn write(&mut self, write: &Write) -> Result<(), LinkError> {
        let request = write.frame(self.next_transaction()).map_err(LinkError::Reply)?;
        self.exchange(request, "write").await.map(|_| ())
    }

    fn next_transaction(&mut self) -> u16 {
        let transaction = self.transaction;
        self.transaction = self.transaction.wrapping_add(1);
        transaction
    }

    async fn exchange(&mut self, request: Frame, kind: &'static str) -> Result<Frame, LinkError> {
        let bytes = request.encode().map_err(LinkError::Reply)?;
        self.socket
            .send_to(&bytes, SocketAddr::V4(self.peer))
            .await
            .map_err(|e| LinkError::Io(e.to_string()))?;
        let deadline = tokio::time::Instant::now() + self.timeout;
        for _ in 0..timing::RECEIVE_ATTEMPTS {
            let received =
                tokio::time::timeout_at(deadline, self.socket.recv_from(&mut self.buffer)).await;
            let Ok(received) = received else { break };
            let (length, from) = received.map_err(|e| LinkError::Io(e.to_string()))?;
            if from != SocketAddr::V4(self.peer) {
                tracing::debug!(%from, "datagram from an unexpected peer discarded");
                continue;
            }
            match Frame::decode(&self.buffer[..length], Direction::Response) {
                Ok(reply) if request.check_response(&reply).is_ok() => return Ok(reply),
                Ok(reply) => {
                    tracing::debug!(
                        transaction = reply.transaction,
                        "reply to another request discarded"
                    );
                }
                Err(error) => return Err(LinkError::Reply(error)),
            }
        }
        Err(LinkError::NoReply(kind, request.address))
    }
}
