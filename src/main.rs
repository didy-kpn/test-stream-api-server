use async_std::net::{SocketAddr, TcpListener, TcpStream};
use async_std::sync::{channel, TryRecvError};
use async_tungstenite::{
    accept_async,
    tungstenite::{Error, Message, Result},
};
use futures::prelude::*;
use log::{error, info};

use serde_json::{from_str, Value};

use std::thread::sleep;
use std::time;

#[derive(Clone, Copy, Debug)]
enum Channel {
    Executes,
    Board,
    SnapshotBoard,
}

enum ThreadMEssage {
    Close,
    Error(i64),
    SwitchChannel(bool, Channel),
}

#[derive(Clone, Copy, Debug)]
struct Channels {
    executes: bool,
    board: bool,
    snapshot_board: bool,
}

impl Channels {
    fn default() -> Self {
        Channels {
            executes: false,
            board: false,
            snapshot_board: false,
        }
    }

    fn set_channels(&mut self, switch: bool, channel: Channel) {
        match channel {
            Channel::Executes => self.executes = switch,
            Channel::Board => self.board = switch,
            Channel::SnapshotBoard => self.snapshot_board = switch,
        }
    }

    fn executes(&self) -> bool {
        self.executes
    }

    fn board(&self) -> bool {
        self.board
    }

    fn snapshot_board(&self) -> bool {
        self.snapshot_board
    }
}

async fn accept_connection(peer: SocketAddr, stream: TcpStream) {
    if let Err(e) = handle_connection(peer, stream).await {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => (),
            err => error!("Error processing connection: {}", err),
        }
    }
}

async fn handle_connection(peer: SocketAddr, stream: TcpStream) -> Result<()> {
    let (tx, rx) = channel(1);
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    let (mut outgoing, mut incoming) = ws_stream.split();

    info!("New WebSocket connection: {}", peer);

    let child = async_std::task::spawn(async move {
        let mut channels = Channels::default();

        loop {
            sleep(time::Duration::from_millis(1000));

            let rx_recv = rx.try_recv();
            if let Err(TryRecvError::Disconnected) = rx.try_recv() {
                println!("{:?}", TryRecvError::Disconnected);
                break;
            }

            if let Ok(tm) = rx_recv {
                match tm {
                    ThreadMEssage::Close => break,
                    ThreadMEssage::Error(no) => {
                        outgoing
                            .send(Message::Text(format!("{{\"error\":{}}}", no)))
                            .await;
                        break;
                    }
                    ThreadMEssage::SwitchChannel(s, c) => {
                        channels.set_channels(s, c);
                    }
                };
            }

            if channels.executes() {
                outgoing.send(Message::text("{\"jsonrpc\":\"2.0\",\"method\":\"channelMessage\",\"params\":{\"channel\":\"test_executes\",\"message\":[]}}")).await;
            }
            if channels.board() {
                outgoing.send(Message::text("{\"jsonrpc\":\"2.0\",\"method\":\"channelMessage\",\"params\":{\"channel\":\"test_board\",\"message\":{}}}")).await;
            }
            if channels.snapshot_board() {
                outgoing.send(Message::text("{\"jsonrpc\":\"2.0\",\"method\":\"channelMessage\",\"params\":{\"channel\":\"test_snapshot_board\",\"message\":{}}}")).await;
            }
        }
    });

    while let Some(msg) = incoming.next().await {
        match msg? {
            Message::Text(text) => {
                let value: Value = from_str(&text).unwrap();

                if !value.is_object() {
                    tx.send(ThreadMEssage::Error(1)).await;
                    break;
                }

                if !value["jsonrpc"].is_string()
                    || value["jsonrpc"].as_str().unwrap() != String::from("2.0")
                {
                    tx.send(ThreadMEssage::Error(2)).await;
                    break;
                }

                if !value["method"].is_string() {
                    tx.send(ThreadMEssage::Error(3)).await;
                    break;
                }

                let switch = match value["method"].as_str().unwrap() {
                    "subscribe" => Some(true),
                    "unsubscribe" => Some(false),
                    _ => None,
                };

                if switch.is_none() {
                    tx.send(ThreadMEssage::Error(4)).await;
                    break;
                }
                let switch = switch.unwrap();

                if !value["params"].is_object() || !value["params"]["channel"].is_string() {
                    tx.send(ThreadMEssage::Error(5)).await;
                    break;
                }

                match value["params"]["channel"].as_str().unwrap() {
                    "test_executes" => {
                        tx.send(ThreadMEssage::SwitchChannel(switch, Channel::Executes))
                            .await;
                    }
                    "test_board" => {
                        tx.send(ThreadMEssage::SwitchChannel(switch, Channel::Board))
                            .await;
                    }
                    "test_snapshot_board" => {
                        tx.send(ThreadMEssage::SwitchChannel(switch, Channel::SnapshotBoard))
                            .await;
                    }
                    _ => {
                        tx.send(ThreadMEssage::Error(6)).await;
                        break;
                    }
                }
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Close(_) => {}
        }
    }

    println!("close");
    tx.send(ThreadMEssage::Close).await;
    child.await;

    Ok(())
}

async fn run() {
    env_logger::init();

    let addr = "127.0.0.1:9002";
    let listener = TcpListener::bind(&addr).await.expect("Can't listen");
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let peer = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        info!("Peer address: {}", peer);

        async_std::task::spawn(accept_connection(peer, stream));
    }
}

fn main() {
    async_std::task::block_on(run());
}
