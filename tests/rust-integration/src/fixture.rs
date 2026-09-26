//! Deterministic server setup for the integration-only fixture schema.
use crate::bindings::StatesServer;
use egui_states::server::{self as s, CallbackHandle};
use std::sync::{Arc, Mutex};

pub struct Fixture {
    pub server: StatesServer,
    callbacks: Vec<CallbackHandle>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    pub fn new(port: u16) -> s::Result<Self> {
        let errors = Arc::new(Mutex::new(Vec::new()));
        let on_error = errors.clone();
        let server = StatesServer::with_options(s::ServerOptions {
            version: Some(StatesServer::VERSION_HASH),
            error_handler: Some(Arc::new(move |error| {
                on_error.lock().unwrap().push(error.to_string())
            })),
            ..s::ServerOptions::default()
        })?;
        let states = &server.states.integration;
        states.value.set(7, false)?;
        states.items.set(vec![1, 2], false)?;
        states.cached.set(&[8, 9], false, false, true)?;
        states
            .cached_multi
            .set(91, &[500, 600], false, false, true)?;
        states.wire.set(
            crate::bindings::structs::WirePacket {
                signed: i64::MIN,
                unsigned: u64::MAX,
                text: "žluťoučký 🦀".into(),
                optional: Some(i32::MIN),
                fixed: [0, 32768, u16::MAX],
                nested: crate::bindings::structs::TestStruct2 {
                    enabled: true,
                    level: u16::MAX,
                    name: "中".into(),
                },
                choice: crate::bindings::enums::TestEnum2::Z,
            },
            false,
        )?;
        let on_error = errors.clone();
        let mut callbacks = vec![server.logging.add_logger(s::LogLevel::Error, move |error| {
            on_error.lock().unwrap().push(error.to_string())
        })];
        let callback_value = states.callback_value.clone();
        let on_error = errors.clone();
        callbacks.push(states.value.connect(move |value| {
            if let Err(e) = callback_value.set(value, true) {
                on_error.lock().unwrap().push(e.to_string());
            }
        }));
        let echo = states.wire_echo.clone();
        callbacks.push(states.wire.connect(move |packet| {
            echo.set(packet, true).unwrap();
        }));
        let extra_takes = server.states.data_take.clone();
        let extra_multi = server.states.data_multi_take.clone();
        let st = states.clone();
        let on_error = errors.clone();
        callbacks.push(states.command.connect(move |command| {
            let result = (|| -> s::Result<()> {
                match command {
                    1 => st.value.set(21, true)?,
                    2 => {
                        st.items.add_item(st.value.get()?, true)?;
                        st.map.set_item(9, 900, true)?;
                    }
                    3 => {
                        extra_takes
                            .take_samples
                            .set(&[-1.25, 0.0, 2.5], false, true, false)?;
                        extra_multi
                            .samples
                            .set(513, &[9.5, -2.25], false, true, false)?;
                        extra_multi
                            .nested
                            .buffer
                            .set(24, &[0, u16::MAX], false, true, false)?;
                        st.take.set("payload".into(), false, true)?;
                        st.empty.set((), false, true)?;
                        st.data.set(&[0, 1, 255], false, true, false)?;
                        st.multi.set(7, &[100, 200], false, true, false)?;
                        st.multi.set(42, &[], false, true, false)?;
                    }
                    4 => {
                        st.take.set(String::new(), false, true)?;
                        st.data.set(&[], false, true, false)?;
                    }
                    10..=13 => {
                        match command {
                            10 => st.take.set("first".into(), true, true)?,
                            11 => st.empty.set((), true, true)?,
                            12 => st.data.set(&[1, 2], true, true, false)?,
                            _ => st.multi.set(73, &[100, 200], true, true, false)?,
                        }
                        st.phase.set(1, true)?;
                        match command {
                            10 => st.take.set(String::new(), true, true)?,
                            11 => st.empty.set((), true, true)?,
                            12 => st.data.set(&[], true, true, false)?,
                            _ => st.multi.set(73, &[], true, true, false)?,
                        }
                        st.phase.set(2, true)?;
                    }
                    20 => {
                        st.cached.set(&[31, 32], false, true, true)?;
                        st.cached_multi.remove_index(91, true)?;
                        st.cached_multi.set(513, &[17], false, true, true)?;
                        st.phase.set(20, true)?;
                    }
                    21 => {
                        st.cached.set(&[41], false, true, false)?;
                        st.cached_multi.reset(true)?;
                        st.phase.set(21, true)?;
                    }
                    22 => {
                        st.multi.set(7, &[7], false, true, false)?;
                        st.multi.set(900, &[9], false, true, false)?;
                        st.multi.remove_index(7, true)?;
                        st.phase.set(22, true)?;
                    }
                    23 => {
                        st.multi.reset(true)?;
                        st.phase.set(23, true)?;
                    }
                    24 | 25 => {
                        st.multi.set(7, &[7], true, true, false)?;
                        if command == 24 {
                            st.multi.remove_index(7, true)?;
                        } else {
                            st.multi.reset(true)?;
                        }
                        st.phase.set(24, true)?;
                        st.multi.set(513, &[9], true, true, false)?;
                        st.phase.set(25, true)?;
                    }
                    99 => st.phase.set(99, true)?,
                    _ => panic!("unknown fixture command"),
                }
                Ok(())
            })();
            if let Err(error) = result {
                on_error.lock().unwrap().push(error.to_string());
            }
        }));
        server.start(port, Some(std::net::Ipv4Addr::LOCALHOST), None)?;
        Ok(Self {
            server,
            callbacks,
            errors,
        })
    }

    pub fn assert_no_errors(&self) {
        assert!(self.errors.lock().unwrap().is_empty(), "{:?}", self.errors);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.stop();
        self.callbacks.clear();
    }
}
