use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, RwLock};

use egui_pysync_common::transport::{self, GraphMessage, Operation, ParseError, Precision};

pub(crate) fn read_message(
    head: &[u8],
    stream: &mut TcpStream,
) -> Result<GraphMessage, ParseError> {
    let precision = match head[0] {
        transport::GRAPH_F32 => Precision::F32,
        transport::GRAPH_F64 => Precision::F64,
        _ => return Err(ParseError::Parse("Invalid graph datatype".to_string())),
    };

    let operation = match head[1] {
        transport::GRAPH_ADD => Operation::Add,
        transport::GRAPH_NEW => Operation::New,
        transport::GRAPH_DELETE => Operation::Delete,
        _ => return Err(ParseError::Parse("Invalid graph operation".to_string())),
    };

    let count = u64::from_le_bytes(head[2..10].try_into().unwrap()) as usize;
    let lines = u64::from_le_bytes(head[10..18].try_into().unwrap()) as usize;

    let data = if let Operation::Add | Operation::New = operation {
        let size = u64::from_le_bytes(head[18..26].try_into().unwrap()) as usize;
        let mut data = vec![0; size];
        stream
            .read_exact(&mut data)
            .map_err(|e| ParseError::Connection(e))?;
        Some(data)
    } else {
        None
    };

    Ok(GraphMessage {
        data,
        precision,
        operation,
        count,
        lines,
    })
}

pub(crate) trait GraphUpdate: Sync + Send {
    fn update_graph(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError>;
}

pub trait GraphType: Sync + Send + Clone + Copy {
    fn check(precision: Precision) -> Result<(), ParseError>;
    fn zero() -> Self;
    fn size() -> usize;
}

pub struct ValueGraph<T> {
    _id: u32,
    data: RwLock<Option<Vec<Vec<T>>>>,
}

impl<T: Clone + Copy> ValueGraph<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            data: RwLock::new(None),
        })
    }

    pub fn get(&self) -> Option<Vec<Vec<T>>> {
        self.data.read().unwrap().clone()
    }

    pub fn process(&self, op: impl FnOnce(Option<&Vec<Vec<T>>>)) {
        op(self.data.read().unwrap().as_ref())
    }
}

impl<T: GraphType> GraphUpdate for ValueGraph<T> {
    fn update_graph(&self, head: &[u8], stream: &mut TcpStream) -> Result<(), ParseError> {
        let message = read_message(head, stream)?;
        T::check(message.precision)?;

        match message.operation {
            Operation::Delete => {
                *self.data.write().unwrap() = None;
            }

            Operation::New => {
                if message.data.is_none() {
                    return Err(ParseError::Parse("No data for new graph".to_string()));
                }
                let msg_data = message.data.unwrap();

                if message.lines * message.count * T::size() != msg_data.len() {
                    return Err(ParseError::Parse(
                        "Invalid data size for new graph".to_string(),
                    ));
                }

                let mut data = Vec::with_capacity(message.lines);
                let mut ptr = msg_data.as_ptr();
                let line_size = message.count * T::size();
                for _ in 0..message.lines {
                    let mut line = vec![T::zero(); message.count];
                    let line_t = line.as_mut_ptr() as *mut u8;
                    unsafe {
                        std::ptr::copy_nonoverlapping(ptr, line_t, line_size);
                        ptr = ptr.add(line_size);
                    }
                    data.push(line);
                }
                *self.data.write().unwrap() = Some(data);
            }

            Operation::Add => {
                if message.data.is_none() {
                    return Err(ParseError::Parse("No data for new graph".to_string()));
                }
                let mut msg_data = message.data.unwrap();

                if message.lines * message.count * T::size() != msg_data.len() {
                    return Err(ParseError::Parse(
                        "Invalid data size for new graph".to_string(),
                    ));
                }

                let mut w = self.data.write().unwrap();
                if w.is_none() {
                    return Err(ParseError::Parse("Graph not initialized".to_string()));
                }

                if w.as_ref().unwrap().len() != message.lines {
                    return Err(ParseError::Parse("Invalid lines count".to_string()));
                }

                let ptr = msg_data.as_mut_ptr() as *mut T;
                let data = w.as_mut().unwrap();
                let line_size = message.count * T::size();
                for line in data {
                    let line_t = unsafe { std::slice::from_raw_parts_mut(ptr, line_size) };
                    line.extend_from_slice(&line_t);
                }
            }
        }

        Ok(())
    }
}

impl GraphType for f32 {
    fn check(precision: Precision) -> Result<(), ParseError> {
        if precision != Precision::F32 {
            return Err(ParseError::Parse(
                "Invalid precision for f32 graph".to_string(),
            ));
        }
        Ok(())
    }

    #[inline]
    fn zero() -> Self {
        0.0
    }

    #[inline]
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl GraphType for f64 {
    fn check(precision: Precision) -> Result<(), ParseError> {
        if precision != Precision::F64 {
            return Err(ParseError::Parse(
                "Invalid precision for f64 graph".to_string(),
            ));
        }
        Ok(())
    }

    #[inline]
    fn zero() -> Self {
        0.0
    }

    #[inline]
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }
}
