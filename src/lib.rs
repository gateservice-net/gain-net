// Copyright (c) 2021 Timo Savola. All rights reserved.
// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE file.

//! Implement TLS servers.

#[macro_use]
extern crate lazy_static;

// The schema file can be found at https://savo.la/gate/listener
#[allow(unused, unused_imports)]
#[path = "listener_generated.rs"]
mod flat;

use flatbuffers::{get_root, FlatBufferBuilder};
use gain::service::Service;
use gain::stream::{CloseStream, Recv, RecvOnlyStream, RecvStream, RecvWriteStream};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

const ACCEPT_SIZE: usize = flat::AcceptSize::Basic as usize;

lazy_static! {
    static ref SERVICE: Service = Service::register("savo.la/gate/listener");
}

/// Binding options.
pub struct BindOptions<'a> {
    _internal: (),

    /// Listening port.
    pub port: u16,

    /// Server name prefix.
    pub prefix: Option<&'a str>,
}

impl<'a> BindOptions<'a> {
    /// Default binding options.
    pub fn new(port: u16) -> Self {
        Self {
            _internal: (),
            port,
            prefix: None,
        }
    }

    /// Opt for a more descriptive server name.
    pub fn with_prefix(prefix: &'a str, port: u16) -> Self {
        Self {
            _internal: (),
            port,
            prefix: Some(prefix),
        }
    }
}

/// Listener address.
pub struct Binding {
    /// Fully-qualified DNS name of the server.
    pub hostname: String,

    /// The listening port.
    pub port: u16,
}

/// Connection listener.
pub struct Listener {
    stream: RecvStream,
    pub addr: Binding,
}

impl Listener {
    /// Listen to TLS connections at `BindOptions::port`.  The fully-qualified
    /// DNS name can be discovered from the `Listener::addr.hostname` field.
    ///
    /// If specified, `BindOptions::prefix` is prepended to the server name.
    /// Its length must be between 1 and 31 characters (inclusive), and it must
    /// consist of lowercase alphanumeric ASCII characters and dash (`-`).  It
    /// must not start or end with a dash.  It must not start with `xn--`.
    pub async fn bind_tls(opt: BindOptions<'_>) -> Result<Self, BindError> {
        let mut b = FlatBufferBuilder::new();

        let prefix = match opt.prefix {
            Some(s) => Some(b.create_string(s)),
            None => None,
        };

        let function = flat::BindTLS::create(
            &mut b,
            &flat::BindTLSArgs {
                accept_size: flat::AcceptSize::Basic,
                name: prefix,
                port: opt.port,
            },
        );

        let call = flat::Call::create(
            &mut b,
            &flat::CallArgs {
                function_type: flat::Function::BindTLS,
                function: Some(function.as_union_value()),
            },
        );

        b.finish_minimal(call);

        SERVICE
            .call(b.finished_data(), |reply: &[u8]| {
                if reply.is_empty() {
                    return Err(BindError::unsupported_call());
                }

                let r = get_root::<flat::Binding>(reply);

                if r.error() != flat::BindError::None {
                    if r.error() == flat::BindError::InvalidAcceptSize {
                        panic!("invalid accept size");
                    }
                    return Err(BindError::new(r.error()));
                }

                let service = SERVICE.input_stream(r.listen_id());

                Ok(Self {
                    stream: service,
                    addr: Binding {
                        hostname: r.host().unwrap().into(),
                        port: r.port(),
                    },
                })
            })
            .await
    }

    /// Accept a client connection.  An `AcceptErrorKind::Closed` error may
    /// occur due to environmental causes.
    pub async fn accept(&mut self) -> Result<Conn, AcceptError> {
        accept(&mut self.stream).await
    }

    /// Detach the closing functionality.  When the `CloseStream` is closed or
    /// dropped, the `Acceptor` will return an `AcceptErrorKind::Closed` error.
    pub fn split(self) -> (Acceptor, CloseStream) {
        let (stream, c) = self.stream.split();
        (
            Acceptor {
                stream,
                addr: self.addr,
            },
            c,
        )
    }
}

/// Connection acceptor.
pub struct Acceptor {
    stream: RecvOnlyStream,
    pub addr: Binding,
}

impl Acceptor {
    /// Accept a client connection.  An `AcceptErrorKind::Closed` error may be
    /// caused by the associated `CloseStream`, or other environmental reasons.
    pub async fn accept(&mut self) -> Result<Conn, AcceptError> {
        accept(&mut self.stream).await
    }
}

async fn accept<R: Recv>(stream: &mut R) -> Result<Conn, AcceptError> {
    let result = Cell::new(Some(Err(AcceptError::listener_closed())));
    let buffer = RefCell::new(Vec::with_capacity(ACCEPT_SIZE));

    let _ = stream
        .recv(ACCEPT_SIZE, |data: &[u8], _: i32| {
            let mut b = buffer.borrow_mut();
            b.extend_from_slice(data);

            let more = ACCEPT_SIZE - b.len();
            if more == 0 {
                let r = get_root::<flat::Accept>(b.as_slice()).basic().unwrap();

                result.set(Some(if r.error() == flat::AcceptError::None {
                    let stream = SERVICE.stream(r.conn_id());

                    let ip = r.addr();
                    let addr = if ip.b() == 0 && ip.c() == 0 && ip.d() == 0 {
                        SocketAddr::V4(SocketAddrV4::new(ip.a().into(), r.port()))
                    } else {
                        let ipv6 = Ipv6Addr::new(
                            (ip.a() >> 16) as u16,
                            (ip.a() >> 0) as u16,
                            (ip.b() >> 16) as u16,
                            (ip.b() >> 0) as u16,
                            (ip.c() >> 16) as u16,
                            (ip.c() >> 0) as u16,
                            (ip.d() >> 16) as u16,
                            (ip.d() >> 0) as u16,
                        );
                        SocketAddr::V6(SocketAddrV6::new(ipv6, r.port(), 0, 0))
                    };

                    Ok(Conn {
                        _internal: (),
                        stream: stream,
                        peer_addr: addr,
                    })
                } else {
                    Err(AcceptError::new(r.error()))
                }));
            }

            more
        })
        .await;

    result.take().unwrap()
}

/// Client connection.
pub struct Conn {
    _internal: (),

    /// I/O stream for exchanging data with the client.
    pub stream: RecvWriteStream,

    /// The client connection's address.
    pub peer_addr: SocketAddr,
}

#[derive(Debug, Eq, PartialEq)]
pub enum BindErrorKind {
    Other,
    TooManyBindings,
    AlreadyBound,
    InvalidName,
    NameTooLong,
    UnsupportedPort,
}

#[derive(Debug)]
pub struct BindError {
    flat: flat::BindError,
}

impl BindError {
    fn new(flat: flat::BindError) -> Self {
        Self { flat }
    }

    fn unsupported_call() -> Self {
        Self::new(flat::BindError::None)
    }

    pub fn kind(&self) -> BindErrorKind {
        match self.flat {
            flat::BindError::TooManyBindings => BindErrorKind::TooManyBindings,
            flat::BindError::AlreadyBound => BindErrorKind::AlreadyBound,
            flat::BindError::InvalidName => BindErrorKind::InvalidName,
            flat::BindError::NameTooLong => BindErrorKind::NameTooLong,
            flat::BindError::UnsupportedPort => BindErrorKind::UnsupportedPort,
            _ => BindErrorKind::Other,
        }
    }

    pub fn as_i16(&self) -> i16 {
        self.flat as i16
    }
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self.kind() {
            BindErrorKind::TooManyBindings => f.write_str("too many bindings"),
            BindErrorKind::AlreadyBound => f.write_str("already bound"),
            BindErrorKind::InvalidName => f.write_str("invalid name"),
            BindErrorKind::NameTooLong => f.write_str("name too long"),
            BindErrorKind::UnsupportedPort => f.write_str("unsupported port"),
            _ => self.as_i16().fmt(f),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum AcceptErrorKind {
    Closed,
    Other,
}

#[derive(Debug)]
pub struct AcceptError {
    flat: flat::AcceptError,
}

impl AcceptError {
    fn new(flat: flat::AcceptError) -> Self {
        Self { flat }
    }

    fn listener_closed() -> Self {
        Self::new(flat::AcceptError::None)
    }

    pub fn kind(&self) -> AcceptErrorKind {
        #[allow(unreachable_patterns)]
        match self.flat {
            flat::AcceptError::None => AcceptErrorKind::Closed,
            _ => AcceptErrorKind::Other,
        }
    }

    pub fn as_i16(&self) -> i16 {
        self.flat as i16
    }
}

impl fmt::Display for AcceptError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self.kind() {
            AcceptErrorKind::Closed => f.write_str("closed"),
            _ => self.as_i16().fmt(f),
        }
    }
}
