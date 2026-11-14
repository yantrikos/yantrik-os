//! A ureq connector that registers its sockets with a [`CutFlag`], so a cut can close them.
//!
//! It opens the TCP connection itself, in the one case the stock connector would open it directly
//! (no proxy for this address), and hands the transport on to the stock proxy, TCP and TLS
//! connectors. With a proxy in the way it steps aside, and the call is bounded as before: by the
//! flag at each line, and by the client timeout.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, LazyBuffers, NextTimeout,
    RustlsConnector, TcpConnector, Transport,
};
use ureq::Error;

use super::cut_flag::CutFlag;

#[derive(Debug)]
struct CutTcp(Arc<CutFlag>);

impl Connector for CutTcp {
    type Out = CutTransport;

    fn connect(&self, details: &ConnectionDetails, _: Option<()>) -> Result<Option<CutTransport>, Error> {
        let through_proxy = details
            .config
            .proxy()
            .is_some_and(|p| !p.is_no_proxy(details.uri));
        if through_proxy {
            return Ok(None);
        }
        let stream = open(details)?;
        if !self.0.register(&stream) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "the provider was removed",
            )));
        }
        let config = details.config;
        let buffers = LazyBuffers::new(config.input_buffer_size(), config.output_buffer_size());
        Ok(Some(CutTransport { stream, buffers }))
    }
}

/// The first address that answers, each given the connect timeout.
fn open(details: &ConnectionDetails) -> Result<TcpStream, Error> {
    let wait = details.timeout.not_zero().map(|t| *t);
    let mut last = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Connection refused");
    for addr in details.addrs.iter() {
        let tried = match wait {
            Some(wait) => TcpStream::connect_timeout(addr, wait),
            None => TcpStream::connect(addr),
        };
        match tried {
            Ok(stream) => {
                stream.set_nodelay(details.config.no_delay())?;
                return Ok(stream);
            }
            Err(e) => last = e,
        }
    }
    Err(Error::Io(last))
}

/// An agent whose sockets `cut` can close.
pub(super) fn agent(config: ureq::config::Config, cut: &Arc<CutFlag>) -> ureq::Agent {
    let connector = CutTcp(cut.clone())
        .chain(ConnectProxyConnector::default())
        .chain(TcpConnector::default())
        .chain(RustlsConnector::default());
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

/// A plain TCP transport: ureq's own is not exported, and this one is the same read and write
/// loop over the socket this module opened.
struct CutTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
}

impl std::fmt::Debug for CutTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CutTransport")
            .field("addr", &self.stream.peer_addr().ok())
            .finish()
    }
}

/// What a socket error is to ureq: a timeout is the call's, anything else is I/O.
fn io_error(e: std::io::Error, timeout: NextTimeout) -> Error {
    use std::io::ErrorKind::{TimedOut, WouldBlock};
    match e.kind() {
        TimedOut | WouldBlock => Error::Timeout(timeout.reason),
        _ => Error::Io(e),
    }
}

impl Transport for CutTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        self.stream.set_write_timeout(timeout.not_zero().map(|t| *t))?;
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output).map_err(|e| io_error(e, timeout))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        self.stream.set_read_timeout(timeout.not_zero().map(|t| *t))?;
        let read = self
            .stream
            .read(self.buffers.input_append_buf())
            .map_err(|e| io_error(e, timeout))?;
        self.buffers.input_appended(read);
        Ok(read > 0)
    }

    fn is_open(&mut self) -> bool {
        true
    }
}
