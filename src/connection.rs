use std::{pin::Pin, sync::Arc, task::Poll};

use libp2p_core::StreamMuxer;
use crate::{stream::{Stream, StreamError}, ConnectionShould};

#[derive(Debug, Clone)]
pub struct Connection {
    connection: iroh::endpoint::Connection,
    connection_should: ConnectionShould,
}


impl StreamMuxer for Connection {
    type Substream = Stream;

    type Error = StreamError;

    // incoming queue: iroh Protocol::accept will push to unbounded tokio channel
    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        todo!()
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        todo!()
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<libp2p_core::muxing::StreamMuxerEvent, Self::Error>> {
        todo!()
    }
}


