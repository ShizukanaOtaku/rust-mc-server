use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener},
    sync::{Arc, RwLock},
    thread,
    time::SystemTime,
};

use mc_protocol::{
    MAX_PACKET_SIZE,
    packet::{
        ConnectionState,
        clientbound::{ClientboundPacket, legacy_server_status},
        data_types::{PrefixedArray, PrefixedOptional, Property},
        parse_packet,
        serverbound::{PacketParseError, ServerboundPacket},
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
        thread::spawn(move || {
            handle_connection(stream, &thread_states);
            println!("Disconnecting a client");
            thread_states
                .write()
                .unwrap()
                .insert(address, ConnectionState::Handshaking);
        });
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

                for raw_packet in packets {
                    let connection_state = *states
                        .read()
                        .unwrap()
                        .get(&peer_addr)
                        .unwrap_or(&ConnectionState::Handshaking);
                    let id = raw_packet.id;
                    let length = raw_packet.length;
                    let data = raw_packet.data.clone();
                    match ServerboundPacket::try_from(connection_state, raw_packet) {
                        Ok(packet) => handle_packet(&mut stream, packet, states),
                        Err(error) => match error {
                            PacketParseError::CorruptPacket { problematic_field } => {
                                println!(
                                    "[{:?}] Corrupt packet received: {length}, {id}, {data:?}\n\t Corrupt field: {problematic_field}",
                                    states.read().unwrap().get(&peer_addr).unwrap()
                                );
                            }
                            PacketParseError::UnknownPacket { id } => {
                                println!(
                                    "Unknown packet type: {id} for state {:?}",
                                    states.read().unwrap().get(&peer_addr).unwrap()
                                )
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
    packet: ServerboundPacket,
    states: &Arc<RwLock<HashMap<IpAddr, ConnectionState>>>,
) {
    let client_address = stream.peer_addr().unwrap().ip();
    println!(
        "Handling {} packet",
        packet.get_name().unwrap_or("an unknown")
    );
    let state = *states
        .read()
        .unwrap()
        .get(&client_address)
        .unwrap_or(&ConnectionState::Handshaking);
    match packet {
        ServerboundPacket::Handshake {
            protocol_version: _,
            server_address: _,
            server_port: _,
            next_state,
        } => {
            let next_state: isize = next_state.try_into().unwrap();
            states.write().unwrap().insert(
                client_address,
                match next_state {
                    1 => ConnectionState::Status,
                    2 | 3 => ConnectionState::Login,
                    _ => ConnectionState::Handshaking,
                },
            );
        }
        ServerboundPacket::StatusRequest {} => send_status(state, stream),
        ServerboundPacket::PingRequest { timestamp: _ } => send_pong(state, stream),
        ServerboundPacket::LegacyServerListPing {} => {
            stream
                .write_all(&legacy_server_status(769, "1.21.4", "RustMC", 8, 64))
                .unwrap();
        }
        ServerboundPacket::LoginStart {
            player_name,
            player_uuid,
        } => {
            send_packet(
                ConnectionState::Status,
                stream,
                ClientboundPacket::LoginSuccess {
                    uuid: player_uuid,
                    username: player_name,
                    properties: PrefixedArray::new(vec![Property(
                        "textures".to_string(),
                        "".to_string(),
                        PrefixedOptional(Some(String::new())),
                    )]),
                },
            );
        }
        ServerboundPacket::LoginAcknowledged {} => {
            println!("{client_address} acknowledged the login, entering configuration");
            states
                .write()
                .unwrap()
                .insert(client_address, ConnectionState::Configuration);
        }
        ServerboundPacket::ClientInformation {
            locale,
            view_distance,
            chat_mode,
            chat_colors,
            displayed_skin_parts,
            main_hand,
            enable_text_filtering,
            allow_server_listings,
            particle_status,
        } => {
            println!(
                "Received client info:
                locale: {locale}
                view_distance: {view_distance} 
                chat_mode: {chat_mode:?}
                chat_colors: {chat_colors}
                displayed_skin_parts: {displayed_skin_parts}
                main_hand: {main_hand:?}
                enable_text_filtering: {enable_text_filtering}
                allow_server_listings: {allow_server_listings} 
                particle_status: {particle_status:?}"
            );
            send_packet(
                ConnectionState::Configuration,
                stream,
                ClientboundPacket::FinishConfiguration {},
            );
        }
        ServerboundPacket::FinishConfiguration {} => {
            println!("{client_address} finished configuration");
        }
        _ => {
            println!("Ignoring the {} packet", packet.get_name().unwrap());
        }
    }
}

fn send_pong(state: ConnectionState, stream: &mut std::net::TcpStream) {
    send_packet(
        state,
        stream,
        ClientboundPacket::PongResponse {
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        },
    );
}

fn send_status(state: ConnectionState, stream: &mut std::net::TcpStream) {
    send_packet(
        state,
        stream,
        ClientboundPacket::StatusResponse {
            json_response: SERVER_STATUS.to_string(),
        },
    );
}

fn send_packet(
    state: ConnectionState,
    stream: &mut std::net::TcpStream,
    packet: ClientboundPacket,
) {
    let bytes: Vec<u8> = packet
        .serialize(state)
        .expect("Failed to serialize a packet");
    stream.write_all(bytes.as_slice()).unwrap();
}
