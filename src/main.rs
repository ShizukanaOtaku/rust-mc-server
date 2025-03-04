use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use mc_protocol::{
    MAX_PACKET_SIZE,
    packet::{
        inbound::{InboundPacket, PacketParseError},
        outbound::OutboundPacket,
        parse_packet,
    },
};

const SERVER_STATUS: &str = "
{
    \"version\": {
            \"name\":\"1.21.4\",
            \"protocol\":769
        }
    }
";

fn main() {
    let listener = TcpListener::bind("127.0.0.1:25565").unwrap();

    for stream in listener.incoming() {
        let mut stream = stream.unwrap();

        thread::spawn(move || {
            handle_connection(&mut stream);
        });
    }
}

fn handle_connection(stream: &mut std::net::TcpStream) {
    let mut buf = vec![0; MAX_PACKET_SIZE];
    let bytes_read = stream.read(&mut buf).unwrap();
    let buf = &buf[0..bytes_read];
    println!("Read {bytes_read} bytes: {buf:?}");
    let raw_packet = parse_packet(&buf.to_vec());
    match InboundPacket::try_from(raw_packet) {
        Ok(packet) => handle_packet(stream, packet),
        Err(error) => match error {
            PacketParseError::CorruptPacket => println!("Corrupt packet received."),
            PacketParseError::UnknownPacket { id } => println!("Unknown packet type: {id}"),
        },
    }
}

fn handle_packet(stream: &mut std::net::TcpStream, packet: InboundPacket) {
    match packet {
        InboundPacket::HandshakePacket {
            protocol_version,
            server_address,
            server_port,
            next_state,
        } => {
            println!(
                "A handshake was received: protocol: {protocol_version}, {server_address}:{server_port}, next_state: {next_state}"
            );
            let response = OutboundPacket::StatusResponsePacket {
                status_json: SERVER_STATUS.to_string(),
            };
            let bytes: Vec<u8> = response.into();
            stream.write_all(bytes.as_slice()).unwrap();
        }
    }
}
