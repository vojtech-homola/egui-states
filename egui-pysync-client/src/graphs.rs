use std::ptr::copy_nonoverlapping;
use std::sync::{Arc, RwLock};

use egui_pysync_transport::graphs::{GraphMessage, Precision};

pub(crate) trait GraphUpdate: Sync + Send {
    fn update_graph(&self, message: GraphMessage) -> Result<(), String>;
}

pub trait GraphType: Sync + Send + Clone + Copy {
    fn check(precision: Precision) -> Result<(), String>;
    fn zero() -> Self;
    fn size() -> usize;
}

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Vec<T>,
    pub changed: bool,
}

impl<T> Graph<T> {
    fn new() -> Self {
        Self {
            y: Vec::new(),
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
    fn update_graph(&self, message: GraphMessage) -> Result<(), String> {
        match message {
            GraphMessage::All(graph) => {
                T::check(graph.precision)?;
                let mut x = vec![T::zero(); graph.points];
                let mut y = vec![T::zero(); graph.points];
                let line_size = graph.points * T::size();

                let mut ptr = graph.data.as_ptr();
                unsafe {
                    copy_nonoverlapping(ptr, x.as_mut_ptr() as *mut u8, line_size);
                    ptr = ptr.add(line_size);
                    copy_nonoverlapping(ptr, y.as_mut_ptr() as *mut u8, line_size);
                }

                let mut g = self.graph.write().unwrap();
                g.x = x;
                g.y = y;
                g.changed = true;

                Ok(())
            }

            GraphMessage::AddPoints(graph) => {
                T::check(graph.precision)?;
                let line_size = graph.points * T::size();
                let mut x = vec![T::zero(); graph.points];
                let mut y = vec![T::zero(); graph.points];
                let mut ptr = graph.data.as_ptr();

                unsafe {
                    copy_nonoverlapping(ptr, x.as_mut_ptr() as *mut u8, line_size);
                    ptr = ptr.add(line_size);
                    copy_nonoverlapping(ptr, y.as_mut_ptr() as *mut u8, line_size);
                }

                let mut g = self.graph.write().unwrap();
                g.x.extend_from_slice(&x);
                g.y.extend_from_slice(&y);
                g.changed = true;

                Ok(())
            }

            GraphMessage::Reset => {
                let mut g = self.graph.write().unwrap();
                g.y.clear();
                g.x.clear();
                g.changed = true;

                Ok(())
            }
        }
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
