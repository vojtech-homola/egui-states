use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::transport::{self, ParseError};

/*
CommandMessage

|1B - type | 1B - command | ... HEAD_SIZE - 2 B rest of the head |

| usize - optional data |
*/

#[inline]
fn write_command_head(head: &mut [u8], command_type: u8) {
    head[0] = transport::TYPE_COMMAND;
    head[1] = command_type;
}

const COM_ERROR: u8 = 0;
const COM_ACK: u8 = 1;
const COM_UPDATE: u8 = 2;
const COM_HANDSHAKE: u8 = 3;

pub enum CommandMessage {
    Error(String),
    Ack(u32),
    Handshake(u64),
    Update(f32),
}

impl CommandMessage {
    pub fn as_str(&self) -> &str {
        match self {
            CommandMessage::Error(_) => "ErrorCommand",
            CommandMessage::Ack(_) => "AckCommand",
            CommandMessage::Handshake(_) => "HandshakeCommand",
            CommandMessage::Update(_) => "UpdateCommand",
        }
    }

    pub fn write_message(&self, head: &mut [u8], stream: &mut TcpStream) -> io::Result<()> {
        match self {
            CommandMessage::Error(v) => {
                write_command_head(head, COM_ERROR);

                // | 8B - usize - size of rest of the message |
                let to_write = v.as_bytes();
                let mut buffer = vec![0u8; to_write.len()];
                head[2..10].copy_from_slice(&(to_write.len() as u64).to_le_bytes());
                buffer[..].copy_from_slice(to_write);
                stream.write_all(&head)?;
                stream.write_all(&buffer)
            }

            CommandMessage::Ack(v) => {
                write_command_head(head, COM_ACK);

                // | 4B - u32 |
                head[2..6].copy_from_slice(&v.to_le_bytes());
                stream.write_all(&head)
            }

            CommandMessage::Update(v) => {
                write_command_head(head, COM_UPDATE);

                // | 4B - f32 |
                head[2..6].copy_from_slice(&v.to_le_bytes());
                stream.write_all(&head)
            }

            CommandMessage::Handshake(v) => {
                write_command_head(head, COM_HANDSHAKE);

                // | 8B - u64 |
                head[2..10].copy_from_slice(&v.to_le_bytes());
                stream.write_all(&head)
            }
        }
    }

    pub fn read_message(
        head: &[u8],
        stream: &mut TcpStream,
    ) -> Result<CommandMessage, ParseError> {
        let command_type = head[1];

        match command_type {
            COM_ERROR => {
                // | 8B - usize - size of rest of the message |
                let size = u64::from_le_bytes(head[2..10].try_into().unwrap());
                if size == 0 {
                    let text = "String in Error message is empty".to_string();
                    return Err(ParseError::Parse(text));
                }
                let mut buffer = vec![0u8; size as usize];
                stream
                    .read_exact(&mut buffer)
                    .map_err(|e| ParseError::Connection(e))?;
                let v = String::from_utf8(buffer).unwrap();
                Ok(CommandMessage::Error(v))
            }

            COM_ACK => {
                // | 4B - u32 |
                let v = u32::from_le_bytes(head[2..6].try_into().unwrap());
                Ok(CommandMessage::Ack(v))
            }

            COM_UPDATE => {
                // | 4B - f32 |
                let v = f32::from_le_bytes(head[2..6].try_into().unwrap());
                Ok(CommandMessage::Update(v))
            }

            COM_HANDSHAKE => {
                // | 8B - u64 |
                let v = u64::from_le_bytes(head[2..10].try_into().unwrap());
                Ok(CommandMessage::Handshake(v))
            }

            _ => Err(ParseError::Parse(format!(
                "Unknown value subtype for command type: {}",
                command_type,
            ))),
        }
    }
}
