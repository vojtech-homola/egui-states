use std::sync::{Arc, RwLock};
use std

use egui_pysync_common::graphs::{GraphsMessage, Precision};

pub(crate) trait GraphUpdate: Sync + Send {
    fn update_graph(&self, message: GraphsMessage) -> Result<(), String>;
}

pub trait GraphType: Sync + Send + Clone + Copy {
    fn check(precision: Precision) -> Result<(), String>;
    fn zero() -> Self;
    fn size() -> usize;
}

#[derive(Clone)]
pub struct Graph<T> {
    pub data: Vec<Vec<T>>,
    pub x: Vec<T>,
    pub changed: bool,
}

impl<T> Graph<T> {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            x: Vec::new(),
            changed: true,
        }
    }
}

pub struct ValueGraph<T> {
    _id: u32,
    graph: RwLock<Graph<T>>,
}

impl<T: Clone + Copy> ValueGraph<T> {
    pub(crate) fn new(id: u32) -> Arc<Self> {
        Arc::new(Self {
            _id: id,
            graph: RwLock::new(Graph::new()),
        })
    }

    pub fn get(&self) -> Graph<T> {
        self.graph.read().unwrap().clone()
    }

    pub fn process(&self, op: impl Fn(&Graph<T>)) {
        let mut g = self.graph.write().unwrap();
        op(&*g);
        g.changed = false;
    }
}

impl<T: GraphType> GraphUpdate for ValueGraph<T> {
    fn update_graph(&self, message: GraphsMessage) -> Result<(), String> {
        match message {
            GraphsMessage::All(graph) => {
                let mut x = vec![T::zero(); graph.points];
                let line_size = graph.points * T::size();

                let mut ptr = graph.data.as_ptr();
                unsafe {

                }




            }
        }

        T::check(message.precision)?;

        match message.operation {
            Operation::Delete => {
                *self.graph.write().unwrap() = None;
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
                *self.graph.write().unwrap() = Some(data);
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

                let mut w = self.graph.write().unwrap();
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
    fn check(precision: Precision) -> Result<(), String> {
        if precision != Precision::F32 {
            return Err("Invalid precision for f32 graph".to_string());
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
    fn check(precision: Precision) -> Result<(), String> {
        if precision != Precision::F64 {
            return Err("Invalid precision for f64 graph".to_string());
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
