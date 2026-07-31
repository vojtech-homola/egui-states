use crate::server_core::signals::LOGGING_ID;

use super::callbacks::CallbackHandle;
use super::state_server::{StateServer, deserialize_bytes};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Debug),
            1 => Some(Self::Info),
            2 => Some(Self::Warning),
            3 => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct LoggingSignal {
    server: StateServer,
}

impl LoggingSignal {
    pub fn new(server: &StateServer) -> Self {
        server.set_signal_to_queue(LOGGING_ID);
        Self {
            server: server.clone(),
        }
    }

    pub fn add_logger(
        &self,
        level: LogLevel,
        logger: impl Fn(String) + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.server.add_raw_callback(LOGGING_ID, move |data| {
            let (raw_level, message) = deserialize_bytes::<(u8, String)>(&data)?;
            if LogLevel::from_id(raw_level) == Some(level) {
                logger(message);
            }
            Ok(())
        })
    }
}
