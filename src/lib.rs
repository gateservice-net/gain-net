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
use gain::stream::buf::{Buf, Read, ReadStream};
use gain::stream::RecvWriteStream;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

pub use flat::{AcceptError, BindError};

const ACCEPT_SIZE: usize = flat::AcceptSize::Basic as usize;

lazy_static! {
    static ref SERVICE: Service = Service::register("savo.la/gate/listener");
}

/// Connection listener.
pub struct Listener {
    stream: ReadStream,

    /// Fully-qualified DNS name of the server.
    pub hostname: String,

    /// The listening port.
    pub port: u16,
}

impl Listener {
    /// Listen to TLS connections at the specified `port`.  The DNS name can be
    /// discovered from the `hostname` field of the created instance.  A custom
    /// prefix may be prepended to the name.
    ///
    /// If `prefix` is specified, its length must be between 1 and 31
    /// characters (inclusive), and it must consist of lowercase alphanumeric
    /// ASCII characters and dash (`-`).  It must not start or end with dash.
    /// It must not start with `xn--`.
    ///
    /// `buflen` specifies the minimum accept queue length.
    pub async fn bind_tls(
        prefix: Option<&str>,
        port: u16,
        buflen: usize,
    ) -> Result<Self, BindError> {
        let bufsize = ACCEPT_SIZE.checked_mul(buflen.max(1)).unwrap();

        let mut b = FlatBufferBuilder::new();

        let name = match prefix {
            Some(s) => Some(b.create_string(s)),
            None => None,
        };

        let function = flat::BindTLS::create(
            &mut b,
            &flat::BindTLSArgs {
                accept_size: flat::AcceptSize::Basic,
                name: name,
                port: port,
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
                let r = get_root::<flat::Binding>(reply);

                if r.error() != BindError::None {
                    return Err(r.error());
                }

                let stream = SERVICE.input_stream(r.listen_id());

                Ok(Self {
                    stream: ReadStream::with_capacity(bufsize, stream),
                    hostname: r.host().unwrap().into(),
                    port: r.port(),
                })
            })
            .await
    }

    /// Accept a client connection.
    pub async fn accept(&mut self) -> Result<Conn, AcceptError> {
        match self
            .stream
            .buf_read(
                ACCEPT_SIZE,
                |buf: &mut Buf| -> Option<Result<Conn, AcceptError>> {
                    let r = get_root::<flat::Accept>(&buf.as_slice()[..ACCEPT_SIZE])
                        .basic()
                        .unwrap();

                    if r.error() != AcceptError::None {
                        return Some(Err(r.error()));
                    }

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

                    buf.consume(ACCEPT_SIZE);

                    Some(Ok(Conn {
                        stream: stream,
                        peer_addr: addr,
                    }))
                },
            )
            .await
            .unwrap()
        {
            Some(x) => x,
            None => Err(AcceptError::None),
        }
    }
}

/// Client connection.
pub struct Conn {
    pub stream: RecvWriteStream,
    pub peer_addr: SocketAddr,
}
