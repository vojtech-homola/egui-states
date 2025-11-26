use std::hash::{DefaultHasher, Hasher};

use serde::{Deserialize, Serialize};

use crate::serialization::ServerHeader;

// graphs -------------------------------------------------------------
#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, PartialOrd)]
pub enum GraphType {
    F32,
    F64,
}

impl GraphType {
    pub fn bytes_size(&self) -> usize {
        match self {
            GraphType::F32 => 4,
            GraphType::F64 => 8,
        }
    }
}

pub trait GraphElement: Clone + Copy + Send + Sync + 'static {
    fn zero() -> Self;
    fn graph_type() -> GraphType;
    fn bytes_size() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl GraphElement for f32 {
    #[inline]
    fn zero() -> Self {
        0.0
    }

    fn graph_type() -> GraphType {
        GraphType::F32
    }
}

impl GraphElement for f64 {
    #[inline]
    fn zero() -> Self {
        0.0
    }
    fn graph_type() -> GraphType {
        GraphType::F64
    }
}

#[derive(Clone)]
pub struct GraphTyped {
    pub y: Vec<u8>,
    pub x: Option<Vec<u8>>,
    pub graph_type: GraphType,
}

impl GraphTyped {
    pub fn to_data(
        &self,
        id: u64,
        graph_id: u16,
        update: bool,
        add_points: Option<usize>,
    ) -> Vec<u8> {
        let mut head_buffer = [0u8; 32];

        let mut size = self.y.len();
        let mut data_offset = 0;
        let graph_header = match add_points {
            Some(points) => {
                let info = GraphDataInfo {
                    graph_type: self.graph_type,
                    points: points as u64,
                    is_linear: self.x.is_none(),
                };
                let header = GraphHeader::AddPoints(graph_id, info);
                data_offset = size - points;
                size = points;
                header
            }
            None => {
                let points = self.y.len() / self.graph_type.bytes_size();
                let info = GraphDataInfo {
                    graph_type: self.graph_type,
                    points: points as u64,
                    is_linear: self.x.is_none(),
                };
                GraphHeader::Set(graph_id, info)
            }
        };
        let header = ServerHeader::Graph(id, update, graph_header);
        let offset = postcard::to_slice(&header, head_buffer[0..].as_mut())
            .expect("Failed to serialize graph data info")
            .len();

        size *= self.graph_type.bytes_size();
        data_offset *= self.graph_type.bytes_size();

        match self.x {
            Some(ref x) => {
                let mut data = vec![0u8; size * 2 + offset];
                data[..offset].copy_from_slice(&head_buffer[..offset]);
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = (x.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset..offset + size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset + size..].copy_from_slice(dat_slice);
                }

                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented.");
                }

                data
            }

            None => {
                let mut data = vec![0u8; size + offset];
                data[..offset].copy_from_slice(&head_buffer[..offset]);
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
                        std::slice::from_raw_parts(ptr, size)
                    };
                    data[offset..].copy_from_slice(dat_slice);
                }

                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented.");
                }

                data
            }
        }
    }
}

// impl<T: GraphElement + Serialize> Graph<T> {
//     pub fn to_data(&self, id: u32, graph_id: u16, add_points: Option<usize>) -> Vec<u8> {
//         // let bytes_size = std::mem::size_of::<T>() * self.y.len();
//         let mut head_buffer = [0u8; 32];

//         head_buffer[1..5].copy_from_slice(&id.to_le_bytes());

//         let mut size = self.y.len();
//         let mut data_offset = 0;
//         let message = match add_points {
//             Some(points) => {
//                 let info = GraphDataInfo::new::<T>(points, self.x.is_none());
//                 let message = GraphHeader::AddPoints(graph_id, info);
//                 data_offset = size - points;
//                 size = points;
//                 message
//             }
//             None => {
//                 let points = self.y.len();
//                 let info = GraphDataInfo::new::<T>(points, self.x.is_none());
//                 let message = GraphHeader::Set(graph_id, info);
//                 message
//             }
//         };
//         let offset = postcard::to_slice(&message, head_buffer[5..].as_mut())
//             .expect("Failed to serialize graph data info")
//             .len()
//             + 5;

//         size *= std::mem::size_of::<T>();
//         data_offset *= std::mem::size_of::<T>();

//         match self.x {
//             Some(ref x) => {
//                 let mut data = vec![0u8; size * 2 + offset];
//                 data[..offset].copy_from_slice(&head_buffer[..offset]);
//                 #[cfg(target_endian = "little")]
//                 {
//                     let dat_slice = unsafe {
//                         let ptr = (x.as_ptr() as *const u8).add(data_offset);
//                         std::slice::from_raw_parts(ptr, size)
//                     };
//                     data[offset..offset + size].copy_from_slice(dat_slice);

//                     let dat_slice = unsafe {
//                         let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
//                         std::slice::from_raw_parts(ptr, size)
//                     };
//                     data[offset + size..].copy_from_slice(dat_slice);
//                 }

//                 #[cfg(target_endian = "big")]
//                 {
//                     unimplemented!("Big endian not implemented.");
//                 }

//                 data
//             }

//             None => {
//                 let mut data = vec![0u8; size + offset];
//                 data[..offset].copy_from_slice(&head_buffer[..offset]);
//                 #[cfg(target_endian = "little")]
//                 {
//                     let dat_slice = unsafe {
//                         let ptr = (self.y.as_ptr() as *const u8).add(data_offset);
//                         std::slice::from_raw_parts(ptr, size)
//                     };
//                     data[offset..].copy_from_slice(dat_slice);
//                 }

//                 #[cfg(target_endian = "big")]
//                 {
//                     unimplemented!("Big endian not implemented.");
//                 }

//                 data
//             }
//         }
//     }
// }

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Option<Vec<T>>,
}

impl<T: GraphElement> Graph<T> {
    pub fn add_points_from_data(&mut self, info: GraphDataInfo, data: &[u8]) -> Result<(), String> {
        let GraphDataInfo {
            graph_type,
            points,
            is_linear,
        } = info;
        let points = points as usize;

        if graph_type != T::graph_type() {
            return Err("Incoming Graph data type does not match.".to_string());
        }

        #[cfg(target_endian = "little")]
        {
            match (&mut self.x, is_linear) {
                (Some(x), false) => {
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
            unimplemented!("Big endian not implemented.");
        }
    }

    pub fn from_graph_data(info: GraphDataInfo, data: &[u8]) -> Result<Self, String> {
        let GraphDataInfo {
            graph_type,
            is_linear,
            points,
        } = info;
        let points = points as usize;

        if T::graph_type() != graph_type {
            return Err("Incoming Graph data type does not match.".to_string());
        }

        #[cfg(target_endian = "little")]
        {
            match is_linear {
                true => {
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let y_ptr = y.as_mut_ptr() as *mut u8;
                    let bytes = points * size_of::<T>();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data.as_ptr(), y_ptr, bytes);
                        y.set_len(points);
                    }

                    Ok(Graph { x: None, y })
                }
                false => {
                    let bytes = points * size_of::<T>();
                    let mut x: Vec<T> = Vec::with_capacity(points);
                    let ptr = x.as_mut_ptr() as *mut u8;
                    let mut data_ptr = data.as_ptr();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        x.set_len(points);
                    }
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let ptr = y.as_mut_ptr() as *mut u8;
                    unsafe {
                        data_ptr = data_ptr.add(bytes);
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        y.set_len(points);
                    }

                    Ok(Graph { x: Some(x), y })
                }
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented.");
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct GraphDataInfo {
    graph_type: GraphType,
    is_linear: bool,
    points: u64,
}

// impl GraphDataInfo {
//     pub fn new(points: usize, is_linear: bool, graph_type: GraphType) -> Self {
//         Self {
//             graph_type,
//             is_linear,
//             points: points as u64,
//         }
//     }
// }

#[derive(Serialize, Deserialize)]
pub enum GraphHeader {
    Set(u16, GraphDataInfo),
    AddPoints(u16, GraphDataInfo),
    Remove(u16),
    Reset,
}

// impl<'a, T: Deserialize<'a>> GraphHeader<T> {
//     pub fn deserialize(data: &'a [u8]) -> Result<(Self, &'a [u8]), String> {
//         postcard::take_from_bytes(data).map_err(|e| e.to_string())
//     }
// }
