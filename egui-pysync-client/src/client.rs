use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::spawn;

use egui_pysync_common::{commands::CommandMessage, transport::ParseError, transport::HEAD_SIZE};

use crate::client_state::UIState;
use crate::states_creator::{ValuesCreator, ValuesList};
use crate::transport::{read_message, ReadResult, WriteMessage};

fn start_gui_client(
    vals: ValuesList,
    mut rx: Receiver<WriteMessage>,
    channel: Sender<WriteMessage>,
    ui_state: UIState,
) {
    let _ = spawn(move || loop {
        // wait for the connection signal
        ui_state.connect_signal().clear();
        ui_state.connect_signal().wait_lock();

        // try to connect to the server
        let res = TcpStream::connect("127.0.0.1:888");
        if res.is_err() {
            continue;
        }

        // get the stream
        let mut stream_write = res.unwrap();
        let mut stream_read = stream_write.try_clone().unwrap();

        // clean mesage queue before starting
        for _v in rx.try_iter() {}

        // read thread -----------------------------------------
        let th_vals = vals.clone();
        let th_ui_state = ui_state.clone();
        let th_channel = channel.clone();
        let recv_tread = spawn(move || {
            let mut head = [0u8; HEAD_SIZE];
            loop {
                // read the message
                let res = read_message(&th_vals, &mut head, &mut stream_read);
                if let Err(e) = res {
                    match e {
                        ParseError::Connection(e) => {
                            println!("Error reading message: {:?}", e); // TODO: log error
                            break;
                        }
                        ParseError::Parse(error) => {
                            th_channel
                                .send(WriteMessage::Command(CommandMessage::Error(error)))
                                .unwrap();
                            continue;
                        }
                    }
                }

                let result = res.unwrap();
                match result {
                    ReadResult::Update(update) => {
                        if update {
                            th_ui_state.update(0.);
                        }
                    }
                    ReadResult::Command(command) => {
                        if let CommandMessage::Update(t) = command {
                            th_ui_state.update(t);
                        }
                    }
                }
            }
        });

        // send thread -----------------------------------------
        let send_thread = spawn(move || {
            // preallocate buffer
            let mut head = [0u8; HEAD_SIZE];

            // send handshake
            let handshake: WriteMessage = WriteMessage::Command(CommandMessage::Handshake(0));
            let res = handshake.write_message(&mut head, &mut stream_write);
            if let Err(e) = res {
                println!("Error for sending hadnskae: {:?}", e); // TODO: log error
                return rx;
            }

            loop {
                // wait for the message from the channel
                let message = rx.recv().unwrap();

                // check if the message is terminate
                if let WriteMessage::Terminate = message {
                    break;
                }

                // send message
                let res = message.write_message(&mut head, &mut stream_write);
                if let Err(e) = res {
                    println!("Error for sending message: {:?}", e); // TODO: log error
                    break;
                }
            }
            rx
        });

        // wait for the read thread to finish
        recv_tread.join().unwrap();

        // terminate the send thread
        channel.send(WriteMessage::Terminate).unwrap();
        rx = send_thread.join().unwrap();
    });
}

pub struct ClientBuilder {
    creator: ValuesCreator,
    channel: Sender<WriteMessage>,
    rx: Receiver<WriteMessage>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        let (channel, rx) = std::sync::mpsc::channel();
        let creator = ValuesCreator::new(channel.clone());

        Self {
            creator,
            channel,
            rx,
        }
    }

    pub fn creator(&mut self) -> &mut ValuesCreator {
        &mut self.creator
    }

    pub fn build(self) -> UIState {
        let Self {
            creator,
            channel,
            rx,
        } = self;

        let values = creator.get_values();
        let ui_state = UIState::new();
        start_gui_client(values, rx, channel, ui_state.clone());

        ui_state
    }
}
