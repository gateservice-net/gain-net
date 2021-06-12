// Copyright (c) 2021 Timo Savola.
// Use of this source code is governed by the MIT
// license that can be found in the LICENSE file.

use chrono::Utc;
use gain::origin;
use gain::stream::buf::{Read as _, ReadStream};
use gain::stream::{Close as _, Write as _, WriteOnlyStream, WriteStream};
use gain::task::block_on;
use gain_listener::{AcceptErrorKind, Acceptor, BindOptions, Listener};
use httparse::{Request, EMPTY_HEADER};
use std::io::{stdout, Write as _};
use std::net::SocketAddr;

fn main() {
    block_on(async {
        let log = origin::accept().await.unwrap();
        let (_, mut log) = log.split();

        let lis = Listener::bind_tls(BindOptions::with_prefix("www", 443))
            .await
            .unwrap();

        log.write(format!("Host: {}\n", lis.addr.hostname).as_bytes())
            .await
            .unwrap();

        let (mut acc, close) = lis.split();

        loop {
            println!("main loop");
            if !handle_conn(&mut log, &mut acc).await {
                break;
            }
        }

        println!("listener closed");
        drop(close);
    });
}

async fn handle_conn(mut log: &mut WriteStream, acc: &mut Acceptor) -> bool {
    let conn = match acc.accept().await {
        Ok(conn) => conn,
        Err(e) => match e.kind() {
            AcceptErrorKind::Closed => return false,
            _ => panic!("{}", e),
        },
    };

    println!("conn accepted");

    let (r, w) = conn.stream.split();
    let (mut w, mut c) = w.split();

    let mut r = ReadStream::new(r);
    let mut buf = Vec::new();
    let mut len = 0;

    loop {
        buf.resize(buf.len() + 64, 0);
        match r.read(&mut buf[len..]).await {
            Ok(n) => len += n,
            Err(e) => {
                println!("read error: {}", e);
                return true;
            }
        }

        let mut headers = [EMPTY_HEADER; 100];
        let mut req = Request::new(&mut headers);
        match req.parse(&buf[..len]) {
            Ok(result) => {
                if !result.is_partial() {
                    handle_request(&mut log, &acc.addr.hostname, &mut w, conn.peer_addr, req).await;
                    break;
                }
            }
            Err(e) => {
                println!("parse error: {}", e);
                stdout().flush().unwrap();
                return true;
            }
        }
    }

    println!("closing conn");
    c.close().await;
    println!("conn closed");
    true
}

async fn handle_request(
    log: &mut WriteStream,
    hostname: &str,
    stream: &mut WriteOnlyStream,
    addr: SocketAddr,
    req: Request<'_, '_>,
) {
    let time = Utc::now();
    let method = req.method.unwrap().to_uppercase();
    let path = req.path.unwrap();
    let code;
    let status;
    let mut content_type = "application/octet-stream".to_string();
    let mut headers = String::new();
    let mut content = Vec::new();

    match method.as_str() {
        "GET" => match path {
            "/" => {
                code = 302;
                status = "Found";
                content_type = "text/html".to_string();
                let location = "/hello";
                headers = format!("Location: {}\r\n", location);
                content = format!("Go to <a href=\"{0}\">{0}</a>\n", location)
                    .as_bytes()
                    .to_vec();
            }

            "/favicon.ico" => {
                code = 200;
                status = "OK";
                content_type = "image/x-icon".to_string();
                content = vec![
                    0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x04, 0x04, 0x02, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0x50, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00,
                    0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x60, 0x00, 0x00, 0x00,
                    0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ];
            }

            "/hello" => {
                code = 200;
                status = "OK";
                content_type = "text/html".to_string();
                content = "<title>gain-listener example</title> <b>Hello, world</b>\n"
                    .as_bytes()
                    .to_vec();
            }

            _ => {
                code = 404;
                status = "Not Found";
            }
        },

        _ => {
            code = 405;
            status = "Method Not Supported";
        }
    }

    if code >= 400 {
        content_type = "text/html".to_string();
        content = format!("<h1>Error</h1> <code>{}</code>\n", status)
            .as_bytes()
            .to_vec();
    }

    stream
        .write(
            format!(
                "HTTP/1.1 {} {}\r
Host: {}\r
Content-Type: {}\r
Content-Length: {}\r
Connection: close\r
Cache-Control: no-cache\r
{}\r\n",
                code,
                status,
                hostname,
                content_type,
                content.len(),
                headers
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    stream.write(&content).await.unwrap();

    log.write(
        format!(
            "{} [{}] {} {} {} {}\n",
            addr.ip(),
            time,
            method,
            path,
            code,
            content.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
}
