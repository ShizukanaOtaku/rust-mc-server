use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener},
    sync::{Arc, RwLock},
    time::SystemTime,
};

use mc_protocol::{
    MAX_PACKET_SIZE,
    packet::{
        ConnectionState,
        inbound::{InboundPacket, PacketParseError},
        outbound::{OutboundPacket, legacy_server_status},
        parse_packet,
    },
};

const SERVER_STATUS: &str = "
{
    \"version\": {
        \"name\":\"1.21.4\",
        \"protocol\":769
    },
    \"description\": {
        \"text\": \"Hello, world!\"
    },
    \"players\": {
        \"max\": 64,
        \"online\": 16
    }
}
";

fn main() {
    let listener = TcpListener::bind("127.0.0.1:25565").unwrap();
    let states = Arc::new(RwLock::new(HashMap::new()));

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        let address = stream.peer_addr().unwrap().ip();
        println!("Handling {address}");
        states
            .write()
            .unwrap()
            .insert(address, ConnectionState::Handshaking);

        let thread_states = Arc::clone(&states);
        handle_connection(stream, &thread_states);
        println!("Disconnecting a client");
        thread_states
            .write()
            .unwrap()
            .insert(address, ConnectionState::Handshaking);
    }
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    states: &Arc<RwLock<HashMap<IpAddr, ConnectionState>>>,
) {
    loop {
        let mut buf = vec![0; MAX_PACKET_SIZE];
        match stream.read(&mut buf) {
            Ok(bytes_read) => {
                if bytes_read == 0 {
                    stream.shutdown(std::net::Shutdown::Both).unwrap();
                    return;
                }

                let mut buf = &buf[..bytes_read];
                let mut packets = Vec::new();

                loop {
                    let packet = parse_packet(&buf.to_vec());
                    if let Some(packet) = packet {
                        packets.push(packet.0);
                        buf = &buf[packet.1..];
                    } else {
                        break;
                    }
                }

                let peer_addr = match stream.peer_addr() {
                    Ok(addr) => addr.ip(),
                    Err(_) => return,
                };

                let connection_state = *states
                    .read()
                    .unwrap()
                    .get(&peer_addr)
                    .unwrap_or(&ConnectionState::Handshaking);

                for raw_packet in packets {
                    match InboundPacket::try_from(connection_state, raw_packet) {
                        Ok(packet) => handle_packet(&mut stream, packet, states),
                        Err(error) => match error {
                            PacketParseError::CorruptPacket => println!("Corrupt packet received."),
                            PacketParseError::UnknownPacket { id } => {
                                println!("Unknown packet type: {id}")
                            }
                        },
                    }
                }
            }
            Err(_) => return,
        };
    }
}

fn handle_packet(
    stream: &mut std::net::TcpStream,
    packet: InboundPacket,
    states: &Arc<RwLock<HashMap<IpAddr, ConnectionState>>>,
) {
    println!(
        "Handling {} packet",
        packet.get_name().unwrap_or("an unknown")
    );
    match packet {
        InboundPacket::Handshake {
            protocol_version: _,
            server_address: _,
            server_port: _,
            next_state,
        } => {
            let next_state: isize = next_state.try_into().unwrap();
            states.write().unwrap().insert(
                stream.peer_addr().unwrap().ip(),
                match next_state {
                    1 => ConnectionState::Status,
                    2 | 3 => ConnectionState::Login,
                    _ => ConnectionState::Handshaking,
                },
            );
        }
        InboundPacket::StatusRequest {} => send_status(stream),
        InboundPacket::PingRequest { timestamp: _ } => send_pong(stream),
        InboundPacket::LegacyServerListPing {} => {
            stream
                .write_all(&legacy_server_status(769, "1.21.4", "RustMC", 8, 64))
                .unwrap();
        }
        InboundPacket::LoginStart {
            player_name,
            player_uuid: _,
        } => {
            send_packet(
                stream,
                OutboundPacket::Disconnect {
                    reason: format!("{{\"text\":\"u r a jerk, {player_name} :3\"}}"),
                },
            );
        }
        _ => {
            println!(
                "Ignoring the packet of id {}, state {:?}",
                packet.get_id(),
                packet.get_state()
            );
        }
    }
}

fn send_pong(stream: &mut std::net::TcpStream) {
    send_packet(
        stream,
        OutboundPacket::PongResponse {
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        },
    );
}

fn send_status(stream: &mut std::net::TcpStream) {
    send_packet(
        stream,
        OutboundPacket::StatusResponse {
            json_response: SERVER_STATUS.to_string(),
        },
    );
}

fn send_packet(stream: &mut std::net::TcpStream, packet: OutboundPacket) {
    let bytes: Vec<u8> = packet.into();
    stream.write_all(bytes.as_slice()).unwrap();
}
