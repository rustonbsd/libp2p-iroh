use std::{fmt::Display, pin::Pin};

use tokio::io::AsyncWrite;

// IrohStream error:
#[derive(Debug, Clone)]
pub struct StreamError {
    kind: StreamErrorKind,
}

#[derive(Debug, Clone)]
pub enum StreamErrorKind {
    Read(String),
    Write(String),
    Connection(String),
}

impl From<std::io::Error> for StreamError {
    fn from(err: std::io::Error) -> Self {
        Self {
            kind: StreamErrorKind::Read(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::ConnectionError> for StreamError {
    fn from(err: iroh::endpoint::ConnectionError) -> Self {
        Self {
            kind: StreamErrorKind::Connection(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::WriteError> for StreamError {
    fn from(err: iroh::endpoint::WriteError) -> Self {
        Self {
            kind: StreamErrorKind::Write(err.to_string()),
        }
    }
}

impl From<iroh::endpoint::ReadError> for StreamError {
    fn from(err: iroh::endpoint::ReadError) -> Self {
        Self {
            kind: StreamErrorKind::Read(err.to_string()),
        }
    }
}


impl From<&str> for StreamError {
    fn from(err: &str) -> Self {
        Self {
            kind: StreamErrorKind::Connection(err.to_string()),
        }
    }
}

impl Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            StreamErrorKind::Read(msg) => write!(f, "IrohStream Read Error: {msg}"),
            StreamErrorKind::Write(msg) => write!(f, "IrohStream Write Error: {msg}"),
            StreamErrorKind::Connection(msg) => {
                write!(f, "IrohStream Connection Error: {msg}")
            }
        }
    }
}

impl std::error::Error for StreamError {}

#[derive(Debug)]
pub struct Stream {
    sender: Option<iroh::endpoint::SendStream>,
    receiver: Option<iroh::endpoint::RecvStream>,
    closing: bool,
}

impl Stream {
    pub fn new(
        sender: iroh::endpoint::SendStream,
        receiver: iroh::endpoint::RecvStream,
    ) -> Result<Self, StreamError> {
        tracing::debug!("Stream::new - Creating new stream wrapper");
        Ok(Self {
            sender: Some(sender),
            receiver: Some(receiver),
            closing: false,
        })
    }
}

impl futures::AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Some(receiver) = &mut self.receiver {
            match Pin::new(receiver).poll_read(cx, buf) {
                std::task::Poll::Ready(Ok(n)) => {
                    if n == 0 {
                        tracing::debug!("Stream::poll_read - EOF reached (0 bytes)");
                    } else {
                        tracing::trace!("Stream::poll_read - Read {} bytes", n);
                    }
                    std::task::Poll::Ready(Ok(n))
                }
                std::task::Poll::Ready(Err(e)) => {
                    tracing::debug!("Stream::poll_read - Read error: {}", e);
                    std::task::Poll::Ready(Err(std::io::Error::other(
                        e,
                    )))
                }
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            tracing::debug!("Stream::poll_read - Stream receiver already closed locally");
            std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stream receiver closed",
            )))
        }
    }
}

impl futures::AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        if let Some(sender) = &mut self.sender {
            match Pin::new(sender).poll_write(cx, buf) {
                std::task::Poll::Ready(Ok(n)) => {
                    tracing::trace!("Stream::poll_write - Wrote {} bytes", n);
                    std::task::Poll::Ready(Ok(n))
                }
                std::task::Poll::Ready(Err(e)) => {
                    // Check if this is a "stopped" error (remote side closed)
                    let err_str = e.to_string();
                    if err_str.contains("stopped") || err_str.contains("error 0") {
                        tracing::debug!("Stream::poll_write - Remote peer closed stream: {}", e);
                    } else {
                        tracing::error!("Stream::poll_write - Write error: {}", e);
                    }
                    std::task::Poll::Ready(Err(std::io::Error::other(
                        e,
                    )))
                }
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            tracing::debug!("Stream::poll_write - Stream sender already closed locally");
            std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stream sender closed",
            )))
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Some(sender) = &mut self.sender {
            match Pin::new(sender).poll_flush(cx) {
                std::task::Poll::Ready(Ok(())) => {
                    tracing::trace!("Stream::poll_flush - Flush successful");
                    std::task::Poll::Ready(Ok(()))
                }
                std::task::Poll::Ready(Err(e)) => {
                    tracing::debug!("Stream::poll_flush - Flush error: {}", e);
                    std::task::Poll::Ready(Err(std::io::Error::other(
                        e,
                    )))
                }
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            tracing::debug!("Stream::poll_flush - Stream sender already closed locally");
            std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stream sender closed",
            )))
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.closing {
            tracing::debug!("Stream::poll_close - Starting to close stream (write side)");
            self.closing = true;
            
            // Finish the sender to signal we're done writing
            if let Some(mut sender) = self.sender.take() {
                if let Err(e) = sender.finish() {
                    tracing::warn!("Stream::poll_close - Error finishing sender: {}", e);
                } else {
                    tracing::debug!("Stream::poll_close - Sender finished successfully");
                }
            }
        }
        tracing::debug!("Stream::poll_close - Write side closed");
        std::task::Poll::Ready(Ok(()))
    }
}
