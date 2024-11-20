use std::mem::size_of;

// graph ----------------------------------------------------------------------
const GRAPH_F32: u8 = 5;
const GRAPH_F64: u8 = 10;

const GRAPH_SET: u8 = 201;
const GRAPH_ADD_POINTS: u8 = 202;
const GRAPH_REMOVE: u8 = 203;
const GRAPH_RESET: u8 = 204;

pub trait WriteGraphMessage: Send + Sync {
    fn write_message(self: Box<Self>, head: &mut [u8]) -> Option<Vec<u8>>;
}
pub trait GraphElement: Clone + Copy + Send + Sync + 'static {
    const DOUBLE: bool;

    fn to_le_bytes(self) -> [u8; 8];
    fn from_le_bytes(bytes: &[u8]) -> Self;
    fn zero() -> Self;
}

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Option<Vec<T>>,
}

impl<T: GraphElement> Graph<T> {
    pub fn to_graph_data(&self, points: Option<usize>) -> GraphData<T> {
        let (bytes_size, ptr_pos, points) = match points {
            Some(points) => {
                if points > self.y.len() {
                    panic!("Points selection is bigger than the graph data.");
                }
                let ptr_pos = size_of::<T>() * (self.y.len() - points);
                (size_of::<T>() * points, ptr_pos, points)
            }
            None => (std::mem::size_of::<T>() * self.y.len(), 0, self.y.len()),
        };

        match self.x {
            Some(ref x) => {
                let mut data = vec![0u8; bytes_size * 2];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = x.as_ptr().add(ptr_pos) as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[..bytes_size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr().add(ptr_pos) as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[bytes_size..].copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                GraphData::new(points, data, false)
            }

            None => {
                let mut data = vec![0u8; bytes_size];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr().add(ptr_pos) as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data.copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                GraphData::new(points, data, true)
            }
        }
    }

    pub fn add_points_from_data(&mut self, graph_data: GraphData<T>) -> Result<(), String> {
        let GraphData {
            points,
            data,
            is_linear,
            ..
        } = graph_data;

        #[cfg(target_endian = "little")]
        {
            match (&mut self.x, is_linear) {
                (Some(ref mut x), false) => {
                    let old_size = x.len();
                    x.resize(old_size + points, T::zero());
                    let mut ptr = data.as_ptr() as *const T;
                    let data_slice = unsafe { std::slice::from_raw_parts(ptr, points) };
                    x[old_size..].copy_from_slice(data_slice);

                    self.y.resize(old_size + points, T::zero());
                    let data_slice = unsafe {
                        ptr = ptr.add(points);
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                (None, true) => {
                    let old_size = self.y.len();
                    self.y.resize(old_size + points, T::zero());
                    let data_slice = unsafe {
                        let ptr = data.as_ptr() as *const T;
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                _ => return Err("Incoming Graph data and graph are not compatible.".to_string()),
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented yet.");
        }
    }

    pub fn from_graph_data(graph_data: GraphData<T>) -> Self {
        let GraphData {
            is_linear,
            points,
            data,
            ..
        } = graph_data;

        #[cfg(target_endian = "little")]
        {
            match is_linear {
                true => {
                    let ptr = data.as_ptr() as *const T;
                    let y = unsafe { std::slice::from_raw_parts(ptr, points) }.to_vec();

                    Graph { x: None, y }
                }
                false => {
                    let ptr = data.as_ptr() as *const T;
                    let x = unsafe { std::slice::from_raw_parts(ptr, points) }.to_vec();
                    let ptr = data[points * size_of::<T>()..].as_ptr() as *const T;
                    let y = unsafe { std::slice::from_raw_parts(ptr, points) }.to_vec();

                    Graph { x: Some(x), y }
                }
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented yet.");
        }
    }
}

#[derive(Clone)]
pub struct GraphData<T> {
    _phantom: std::marker::PhantomData<T>,
    is_linear: bool,
    points: usize,
    data: Vec<u8>,
}

impl<T> GraphData<T> {
    fn new(points: usize, data: Vec<u8>, is_linear: bool) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
            is_linear,
            points,
            data,
        }
    }
}

pub enum GraphMessage<T> {
    Set(u16, GraphData<T>),
    AddPoints(u16, GraphData<T>),
    Remove(u16),
    Reset,
}

fn write_head<T: GraphElement>(head: &mut [u8], graph_data: &GraphData<T>) {
    head[1] = if T::DOUBLE { GRAPH_F64 } else { GRAPH_F32 };

    match graph_data.is_linear {
        true => head[2] = 255,
        false => head[2] = 0,
    }

    head[3..7].copy_from_slice(&(graph_data.points as u32).to_le_bytes());
}

impl<T: GraphElement> WriteGraphMessage for GraphMessage<T> {
    fn write_message(self: Box<Self>, head: &mut [u8]) -> Option<Vec<u8>> {
        match *self {
            GraphMessage::Set(id, graph_data) => {
                head[0] = GRAPH_SET;
                write_head(head, &graph_data);
                head[7..9].copy_from_slice(&id.to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::AddPoints(id, graph_data) => {
                head[0] = GRAPH_ADD_POINTS;
                write_head(head, &graph_data);
                head[7..9].copy_from_slice(&id.to_le_bytes());
                Some(graph_data.data)
            }

            GraphMessage::Remove(id) => {
                head[0] = GRAPH_REMOVE;
                head[7..9].copy_from_slice(&id.to_le_bytes());
                None
            }
            GraphMessage::Reset => {
                head[0] = GRAPH_RESET;
                None
            }
        }
    }
}

fn read_head<T: GraphElement>(
    head: &[u8],
    data: Option<Vec<u8>>,
) -> Result<(bool, usize, Vec<u8>), String> {
    let data_type = head[1];
    let is_linear = head[2] != 0;

    if T::DOUBLE && data_type != GRAPH_F64 || !T::DOUBLE && data_type != GRAPH_F32 {
        return Err(format!("Wrong precision for graph message: {}", data_type));
    }

    let points = u32::from_le_bytes([head[3], head[4], head[5], head[6]]) as usize;
    let data = data.ok_or("No data for graph message.")?;

    Ok((is_linear, points, data))
}

impl<T: GraphElement> GraphMessage<T> {
    pub fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let graph_type = head[0];

        match graph_type {
            GRAPH_SET => {
                let (is_linear, points, data) = read_head::<T>(head, data)?;
                let id = u16::from_le_bytes([head[7], head[8]]);

                Ok(GraphMessage::Set(
                    id,
                    GraphData::new(points, data, is_linear),
                ))
            }

            GRAPH_ADD_POINTS => {
                let (is_linear, points, data) = read_head::<T>(head, data)?;
                let id = u16::from_le_bytes([head[7], head[8]]);

                Ok(GraphMessage::AddPoints(
                    id,
                    GraphData::new(points, data, is_linear),
                ))
            }

            GRAPH_REMOVE => {
                let id = u16::from_le_bytes([head[7], head[8]]);
                Ok(GraphMessage::Remove(id))
            }

            GRAPH_RESET => Ok(GraphMessage::Reset),

            _ => Err(format!("Unknown graph message type: {}", graph_type)),
        }
    }
}

impl GraphElement for f32 {
    const DOUBLE: bool = false;

    #[inline]
    fn to_le_bytes(self) -> [u8; 8] {
        let bytes = self.to_le_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3], 0, 0, 0, 0]
    }

    #[inline]
    fn from_le_bytes(bytes: &[u8]) -> Self {
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    #[inline]
    fn zero() -> Self {
        0.0
    }
}

impl GraphElement for f64 {
    const DOUBLE: bool = true;

    #[inline]
    fn to_le_bytes(self) -> [u8; 8] {
        self.to_le_bytes()
    }

    #[inline]
    fn from_le_bytes(bytes: &[u8]) -> Self {
        f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    #[inline]
    fn zero() -> Self {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_graph_all() {
        let data = vec![0u8; 5 * 2 * std::mem::size_of::<f32>()];
        let graph_data = GraphData::<f32>::new(5, data.clone(), true);

        let mut head = [0u8; HEAD_SIZE];
        let message = Box::new(GraphMessage::Set(42, graph_data.clone()));

        let data = message.write_message(&mut head[4..]);
        assert_eq!(data, Some(vec![0u8; 5 * 2 * std::mem::size_of::<f32>()]));

        let new_message = GraphMessage::<f32>::read_message(&mut head[4..], data).unwrap();

        match new_message {
            GraphMessage::Set(idx, new_graph_data) => {
                assert_eq!(idx, 42);
                assert_eq!(graph_data.data, new_graph_data.data);
                assert_eq!(graph_data.points, new_graph_data.points);
                assert_eq!(graph_data.is_linear, new_graph_data.is_linear);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_reset() {
        let mut head = [0u8; HEAD_SIZE];

        let message = Box::new(GraphMessage::<f32>::Reset);
        let data = message.write_message(&mut head[4..]);
        assert_eq!(data, None);

        let message = GraphMessage::<f32>::read_message(&mut head[4..], data).unwrap();

        match message {
            GraphMessage::Reset => (),
            _ => panic!("Wrong message type."),
        }
    }
}
