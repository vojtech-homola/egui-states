use crate::transport::HEAD_SIZE;

/*
CommandMessage
*/

const COM_ERROR: u8 = 30;
const COM_ACK: u8 = 31;
const COM_UPDATE: u8 = 32;
const COM_HANDSHAKE: u8 = 33;

const SIZE_START: usize = HEAD_SIZE - 1 - 8;

pub enum CommandMessage {
    Error(String),
    Ack(u32),
    Handshake(u64, u64),
    Update(f32),
}

impl CommandMessage {
    pub fn as_str(&self) -> &str {
        match self {
            CommandMessage::Error(_) => "ErrorCommand",
            CommandMessage::Ack(_) => "AckCommand",
            CommandMessage::Handshake(_, _) => "HandshakeCommand",
            CommandMessage::Update(_) => "UpdateCommand",
        }
    }

    pub(crate) fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            CommandMessage::Error(error) => {
                head[0] = COM_ERROR;

                let data = error.as_bytes().to_vec();

                println!("------------------------------------");
                println!("len: {}", head[SIZE_START..].len());

                head[SIZE_START..].copy_from_slice(&(data.len() as u64).to_le_bytes());
                Some(data)
            }

            CommandMessage::Ack(ind) => {
                head[0] = COM_ACK;
                head[1..5].copy_from_slice(&ind.to_le_bytes());
                None
            }

            CommandMessage::Update(time) => {
                head[0] = COM_UPDATE;
                head[1..5].copy_from_slice(&time.to_le_bytes());
                None
            }

            CommandMessage::Handshake(version, hash) => {
                head[0] = COM_HANDSHAKE;
                head[1..9].copy_from_slice(&version.to_le_bytes());
                head[9..17].copy_from_slice(&hash.to_le_bytes());
                None
            }
        }
    }

    pub(crate) fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let command_type = head[0];

        match command_type {
            COM_ERROR => {
                let error = match data {
                    Some(data) => String::from_utf8(data).unwrap(),
                    None => return Err("Error message needs additional data.".to_string()),
                };

                Ok(CommandMessage::Error(error))
            }

            COM_ACK => {
                if data.is_some() {
                    return Err("Ack message do not accept additional data.".to_string());
                }

                let ind = u32::from_le_bytes(head[1..5].try_into().unwrap());
                Ok(CommandMessage::Ack(ind))
            }

            COM_UPDATE => {
                if data.is_some() {
                    return Err("Update message do not accept additional data.".to_string());
                }

                let time = f32::from_le_bytes(head[1..5].try_into().unwrap());
                Ok(CommandMessage::Update(time))
            }

            COM_HANDSHAKE => {
                if data.is_some() {
                    return Err("Handshake message do not accept additional data.".to_string());
                }

                let version = u64::from_le_bytes(head[1..9].try_into().unwrap());
                let hash = u64::from_le_bytes(head[9..17].try_into().unwrap());
                Ok(CommandMessage::Handshake(version, hash))
            }

            _ => Err(format!(
                "Unknown value subtype for command type: {}",
                command_type,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_error() {
        let error = "Error message".to_string();
        let mut head = [0u8; HEAD_SIZE];

        let message = CommandMessage::Error(error.clone());

        let data = message.write_message(&mut head[1..]);
        assert_eq!(data.is_some(), true);

        let message = CommandMessage::read_message(&mut head[1..], data).unwrap();
        assert_eq!(message.as_str(), "ErrorCommand");

        if let CommandMessage::Error(error) = message {
            assert_eq!(error, error);
        } else {
            panic!("Invalid message type.");
        }
    }

    #[test]
    fn test_command_ack() {
        let ind = 123456;
        let mut head = [0u8; HEAD_SIZE];

        let message = CommandMessage::Ack(ind);

        let data = message.write_message(&mut head[1..]);
        assert_eq!(data.is_none(), true);

        let message = CommandMessage::read_message(&mut head[1..], data).unwrap();
        assert_eq!(message.as_str(), "AckCommand");

        if let CommandMessage::Ack(new_ind) = message {
            assert_eq!(ind, new_ind);
        } else {
            panic!("Invalid message type.");
        }
    }

    #[test]
    fn test_command_update() {
        let time = 1234.5678;
        let mut head = [0u8; HEAD_SIZE];

        let message = CommandMessage::Update(time);

        let data = message.write_message(&mut head[1..]);
        assert_eq!(data.is_none(), true);

        let message = CommandMessage::read_message(&mut head[1..], data).unwrap();
        assert_eq!(message.as_str(), "UpdateCommand");

        if let CommandMessage::Update(new_time) = message {
            assert_eq!(time, new_time);
        } else {
            panic!("Invalid message type.");
        }
    }

    #[test]
    fn test_command_handshake() {
        let version = 1234567890;
        let hash = 9876543210;
        let mut head = [0u8; HEAD_SIZE];

        let message = CommandMessage::Handshake(version, hash);

        let data = message.write_message(&mut head[1..]);
        assert_eq!(data.is_none(), true);

        let message = CommandMessage::read_message(&mut head[1..], data).unwrap();
        assert_eq!(message.as_str(), "HandshakeCommand");

        if let CommandMessage::Handshake(new_version, new_hash) = message {
            assert_eq!(version, new_version);
            assert_eq!(hash, new_hash);
        } else {
            panic!("Invalid message type.");
        }
    }
}
