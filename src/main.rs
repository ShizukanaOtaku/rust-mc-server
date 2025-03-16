use std::{
    collections::HashMap,
    error::Error,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
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
    let states = Arc::new(Mutex::new(HashMap::new()));

    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        states
            .lock()
            .unwrap()
            .insert(stream.peer_addr().unwrap(), ConnectionState::Handshaking);

        let thread_states = Arc::clone(&states);
        thread::spawn(move || {
            loop {
                if !handle_connection(&mut stream, &thread_states) {
                    println!("Disconnecting a client");
                    break;
                }
            }
        });
    }
}

fn handle_connection(
    stream: &mut std::net::TcpStream,
    states: &Arc<Mutex<HashMap<SocketAddr, ConnectionState>>>,
) -> bool {
    let mut buf = vec![0; MAX_PACKET_SIZE];
    let bytes_read = match stream.read(&mut buf) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let buf = &buf[..bytes_read];

    if bytes_read == 0 {
        return false;
    }

    let raw_packet = parse_packet(&buf.to_vec());
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(_) => return false,
    };

    let connection_state = states
        .lock()
        .unwrap()
        .get(&peer_addr)
        .unwrap_or(&ConnectionState::Handshaking)
        .clone();

    match InboundPacket::try_from(connection_state, raw_packet) {
        Ok(packet) => handle_packet(stream, packet, states),
        Err(error) => match error {
            PacketParseError::CorruptPacket => println!("Corrupt packet received."),
            PacketParseError::UnknownPacket { id } => println!("Unknown packet type: {id}"),
        },
    }

    true
}

fn handle_packet(
    stream: &mut std::net::TcpStream,
    packet: InboundPacket,
    states: &Arc<Mutex<HashMap<SocketAddr, ConnectionState>>>,
) {
    println!("Handling {} packet", packet.get_name().unwrap_or("unknown"));
    match packet {
        InboundPacket::Handshake {
            protocol_version,
            server_address,
            server_port,
            next_state,
        } => {
            let protocol_version: isize = protocol_version.try_into().unwrap();
            let next_state: isize = next_state.try_into().unwrap();
            println!(
                "A handshake was received: protocol: {protocol_version}, {server_address}:{server_port}, next_state: {next_state}",
            );
            states.lock().unwrap().insert(
                stream.peer_addr().unwrap(),
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
    stream.flush().unwrap();
}
