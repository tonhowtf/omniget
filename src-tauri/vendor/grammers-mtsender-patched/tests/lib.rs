// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

pub const TELEGRAM_TEST_DC_2: &str = "149.154.167.40:443";

/// The default datacenter to connect to for testing.
pub const TELEGRAM_DEFAULT_TEST_DC: &str = TELEGRAM_TEST_DC_2;

#[test]
fn test_invoke_encrypted_method() {
    use grammers_mtproto::transport;
    use grammers_mtsender::connect;
    use grammers_tl_types::{LAYER, enums, functions};
    use simple_logger::SimpleLogger;
    use std::str::FromStr;
    use tokio::runtime;

    let _ = SimpleLogger::new()
        .with_level(log::LevelFilter::Debug)
        .init();

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut sender = connect(
            transport::Full::new(),
            grammers_mtsender::ServerAddr::Tcp {
                address: std::net::SocketAddr::from_str(TELEGRAM_TEST_DC_2).unwrap(),
            },
        )
        .await
        .unwrap();

        let response = sender
            .invoke(&functions::InvokeWithLayer {
                layer: LAYER,
                query: functions::InitConnection {
                    api_id: 1,
                    device_model: "Test".to_string(),
                    system_version: "0.1".to_string(),
                    app_version: "0.1".to_string(),
                    system_lang_code: "en".to_string(),
                    lang_pack: "".to_string(),
                    lang_code: "".to_string(),
                    proxy: None,
                    params: None,
                    query: functions::help::GetNearestDc {},
                },
            })
            .await;

        assert!(matches!(response, Ok(enums::NearestDc::Dc(_))));
    });
}

#[test]
#[cfg(feature = "proxy")]
fn test_connection_through_proxy() {
    use grammers_mtproto::authentication;
    use grammers_mtproto::mtp;
    use grammers_mtproto::transport;
    use grammers_mtsender::Sender;
    use simple_logger::SimpleLogger;
    use socks5_server::{
        Command, Server,
        auth::Password,
        proto::{Address, Reply},
    };
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::io;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::{runtime, task};

    let _ = SimpleLogger::new()
        .with_level(log::LevelFilter::Debug)
        .init();

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let socks5_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks5_addr = socks5_listener.local_addr().unwrap();
        let socks5_server = Server::new(
            socks5_listener,
            Arc::new(Password::new(
                b"grammers".to_vec(),
                b"6772616d6d657273".to_vec(),
            )),
        );

        let socks5_task = task::spawn(async move {
            let (conn, _) = socks5_server.accept().await.unwrap();
            let (conn, _) = conn.authenticate().await.unwrap();
            let command = conn.wait().await.unwrap();
            match command {
                Command::Connect(connect, addr) => {
                    let mut target = match addr {
                        Address::SocketAddress(addr) => TcpStream::connect(addr).await.unwrap(),
                        _ => unimplemented!(),
                    };
                    let mut conn = connect
                        .reply(Reply::Succeeded, Address::unspecified())
                        .await
                        .unwrap();
                    io::copy_bidirectional(&mut target, &mut conn)
                        .await
                        .unwrap();
                }
                _ => unimplemented!(),
            }
        });

        let mut sender = Sender::connect(
            transport::Full::new(),
            mtp::Plain::new(),
            grammers_mtsender::ServerAddr::Proxied {
                address: std::net::SocketAddr::from_str(TELEGRAM_TEST_DC_2).unwrap(),
                proxy: format!("socks5://grammers:6772616d6d657273@{socks5_addr}"),
            },
        )
        .await
        .unwrap();

        // Don't need to run the entire flow, just prove that we can send and receive data.
        let (request, data) = authentication::step1().unwrap();
        let response = sender.invoke(&request).await.unwrap();
        authentication::step2(data, response).unwrap();

        drop(sender);
        socks5_task.await.unwrap(); // also make sure proxy finished handling a single connection cleanly
    });
}
