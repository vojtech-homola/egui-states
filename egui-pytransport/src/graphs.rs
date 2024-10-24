// graph ----------------------------------------------------------------------
/*
data:
| f32 | * count
| f64 | * count
---------

graph add:
| 1B - subtype | 1B - precision | 1B - graph subtype | 4B - u32 points | ... |4B - u32 data size |



---------
---------
graph all:
| 1B - subtype | 1B - precision | 8B - u64 points | ... |8B - u64 data size |

---------
graph add points:
| 1B - subtype | 1B - precision | 8B - u64 points | ... |8B - u64 data size |

---------
graph reset:
| 1B - subtype |

*/
const GRAPH_F32: u8 = 5;
const GRAPH_F64: u8 = 10;

const GRAPH_ADD: u8 = 200;
const GRAPH_ADD_POINTS: u8 = 201;
const GRAPH_SET: u8 = 202;
const GRAPH_RESET: u8 = 203;

pub trait WriteGraphMessage {
    fn write_message(self, head: &mut [u8]) -> Option<Vec<u8>>;
}
pub trait GraphElement: Clone + Copy + Send + Sync {
    const DOUBLE: bool;

    fn to_le_bytes(self) -> [u8; 8];
    fn from_le_bytes(bytes: &[u8]) -> Self;
    fn zero() -> Self;
}

#[derive(Clone)]
pub struct GraphLine<T> {
    pub x: Vec<T>,
    pub y: Vec<T>,
}

#[derive(Clone)]
pub struct GraphLinear<T> {
    pub y: Vec<T>,
    pub range: [T; 2],
}

#[derive(Clone)]
pub enum Graph<T> {
    Line(GraphLine<T>),
    Linear(GraphLinear<T>),
}

impl<T: GraphElement> Graph<T> {
    pub fn to_message(&self) -> GraphMessage<T> {
        match self {
            Graph::Line(graph) => {
                let bytes_size = std::mem::size_of::<T>() * graph.x.len();
                let mut data = vec![0u8; bytes_size * 2];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        std::slice::from_raw_parts(graph.x.as_ptr() as *const u8, bytes_size)
                    };
                    data[..bytes_size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        std::slice::from_raw_parts(graph.y.as_ptr() as *const u8, bytes_size)
                    };
                    data[bytes_size..].copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                GraphMessage::Add(GraphsData {
                    range: None,
                    points: graph.x.len(),
                    data,
                })
            }

            Graph::Linear(graph) => {
                let bytes_size = std::mem::size_of::<T>() * graph.y.len();
                let mut data = vec![0u8; bytes_size];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        std::slice::from_raw_parts(graph.y.as_ptr() as *const u8, bytes_size)
                    };
                    data.copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                GraphMessage::Add(GraphsData {
                    range: Some(graph.range),
                    points: graph.y.len(),
                    data,
                })
            }
        }
    }
}

// #[derive(PartialEq, Copy, Clone, Debug)]
// pub enum Precision {
//     F32,
//     F64,
// }

// impl Precision {
//     #[inline]
//     fn to_u8(&self) -> u8 {
//         match self {
//             Precision::F32 => GRAPH_F32,
//             Precision::F64 => GRAPH_F64,
//         }
//     }

//     #[inline]
//     const fn size(&self) -> usize {
//         match self {
//             Precision::F32 => std::mem::size_of::<f32>(),
//             Precision::F64 => std::mem::size_of::<f64>(),
//         }
//     }
// }

#[derive(Clone)]
pub struct GraphsData<T> {
    range: Option<[T; 2]>,
    points: usize,
    data: Vec<u8>,
}

pub enum GraphMessage<T> {
    Add(GraphsData<T>),
    AddPoints(u16, GraphsData<T>),
    Set(u16, GraphsData<T>),
    Reset,
}

fn write_head<T: GraphElement>(head: &mut [u8], graph_data: &GraphsData<T>) {
    let mut flag = if T::DOUBLE { GRAPH_F64 } else { GRAPH_F32 };

    match graph_data.range {
        Some(range) => {
            head[2..10].copy_from_slice(&range[0].to_le_bytes());
            head[10..18].copy_from_slice(&range[1].to_le_bytes());
        }
        None => {
            flag += 128;
        }
    }

    head[1] = flag;
    head[18..22].copy_from_slice(&(graph_data.points as u32).to_le_bytes());
}

impl<T: GraphElement> WriteGraphMessage for GraphMessage<T> {
    fn write_message(self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            GraphMessage::Add(graph_data) => {
                head[0] = GRAPH_ADD;
                write_head(head, &graph_data);
                Some(graph_data.data)
            }
            GraphMessage::AddPoints(id, graph_data) => {
                head[0] = GRAPH_ADD_POINTS;
                write_head(head, &graph_data);
                head[22..24].copy_from_slice(&id.to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::Set(id, graph_data) => {
                head[0] = GRAPH_SET;
                write_head(head, &graph_data);
                head[22..24].copy_from_slice(&id.to_le_bytes());
                Some(graph_data.data)
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
) -> Result<(Option<[T; 2]>, usize, Vec<u8>), String> {
    let mut flag = head[1];

    let range = if flag < 127 {
        Some([
            T::from_le_bytes(&head[2..10]),
            T::from_le_bytes(&head[10..18]),
        ])
    } else {
        flag -= 128;
        None
    };

    if T::DOUBLE && flag != GRAPH_F64 || !T::DOUBLE && flag != GRAPH_F32 {
        return Err(format!("Wrong precision for graph message: {}", flag));
    }

    let points = u32::from_le_bytes([head[18], head[19], head[20], head[21]]) as usize;
    let data = data.ok_or("No data for graph message.")?;

    Ok((range, points, data))
}

impl<T: GraphElement> GraphMessage<T> {
    pub(crate) fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let graph_type = head[0];

        match graph_type {
            GRAPH_ADD => {
                let (range, points, data) = read_head(head, data)?;

                Ok(GraphMessage::Add(GraphsData {
                    range,
                    points,
                    data,
                }))
            }

            GRAPH_ADD_POINTS => {
                let (range, points, data) = read_head(head, data)?;
                let id = u16::from_le_bytes([head[22], head[23]]);

                Ok(GraphMessage::AddPoints(
                    id,
                    GraphsData {
                        range,
                        points,
                        data,
                    },
                ))
            }

            GRAPH_SET => {
                let (range, points, data) = read_head(head, data)?;
                let id = u16::from_le_bytes([head[22], head[23]]);

                Ok(GraphMessage::Set(
                    id,
                    GraphsData {
                        range,
                        points,
                        data,
                    },
                ))
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
        let graph_data = GraphsData::<f32> {
            range: None,
            points: 5,
            data,
        };

        let mut head = [0u8; HEAD_SIZE];
        let message = GraphMessage::Add(graph_data.clone());

        let data = message.write_message(&mut head[6..]);
        assert_eq!(data, Some(vec![0u8; 5 * 2 * std::mem::size_of::<f32>()]));

        let new_message = GraphMessage::read_message(&mut head[4..], data).unwrap();

        match new_message {
            GraphMessage::Add(new_graph_data) => {
                assert_eq!(graph_data.data, new_graph_data.data);
                assert_eq!(graph_data.points, new_graph_data.points);
                assert_eq!(graph_data.range, new_graph_data.range);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_reset() {
        let mut head = [0u8; HEAD_SIZE];

        let message = GraphMessage::<f32>::Reset;
        let data = message.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let message = GraphMessage::<f32>::read_message(&mut head[6..], data).unwrap();

        match message {
            GraphMessage::Reset => (),
            _ => panic!("Wrong message type."),
        }
    }
}
