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
const GRAPH_F32: u8 = 60;
const GRAPH_F64: u8 = 61;

const GRAPH_ALL: u8 = 200;
const GRAPH_ADD_POINTS: u8 = 201;
const GRAPH_RESET: u8 = 204;

pub struct GraphLine<T> {
    pub x: Vec<T>,
    pub y: Vec<T>,
}

pub struct GraphLinear<T> {
    pub y: Vec<T>,
    pub range: [T; 2],
}

pub enum Graph<T> {
    Line(GraphLine<T>),
    Linear(GraphLinear<T>),
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum Precision {
    F32,
    F64,
}

impl Precision {
    #[inline]
    fn to_u8(&self) -> u8 {
        match self {
            Precision::F32 => GRAPH_F32,
            Precision::F64 => GRAPH_F64,
        }
    }

    #[inline]
    const fn size(&self) -> usize {
        match self {
            Precision::F32 => std::mem::size_of::<f32>(),
            Precision::F64 => std::mem::size_of::<f64>(),
        }
    }
}

#[derive(Clone)]
pub struct GraphsData {
    pub precision: Precision,
    pub points: usize,
    pub data: Vec<u8>,
}

pub enum GraphMessage {
    All(GraphsData),
    AddPoints(GraphsData),
    Reset,
}

impl GraphMessage {
    pub(crate) fn write_message(self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            GraphMessage::All(graph_data) => {
                head[0] = GRAPH_ALL;
                head[1] = graph_data.precision.to_u8();

                head[2..10].copy_from_slice(&(graph_data.points as u64).to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::AddPoints(graph_data) => {
                head[0] = GRAPH_ADD_POINTS;
                head[1] = graph_data.precision.to_u8();

                head[2..10].copy_from_slice(&(graph_data.points as u64).to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::Reset => {
                head[0] = GRAPH_RESET;
                None
            }
        }
    }

    pub(crate) fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let graph_type = head[0];

        match graph_type {
            GRAPH_ALL | GRAPH_ADD_POINTS => {
                let precision = match head[1] {
                    GRAPH_F32 => Precision::F32,
                    GRAPH_F64 => Precision::F64,
                    _ => return Err("Unknown precision".to_string()),
                };

                let points = u64::from_le_bytes(head[2..10].try_into().unwrap()) as usize;

                let data = data.ok_or("Graph data is missing.".to_string())?;
                if points * 2 * precision.size() != data.len() {
                    return Err("Invalid data size for graph.".to_string());
                }

                let graphs_data = GraphsData {
                    precision,
                    points,
                    data,
                };

                Ok(match graph_type {
                    GRAPH_ALL => GraphMessage::All(graphs_data),
                    GRAPH_ADD_POINTS => GraphMessage::AddPoints(graphs_data),
                    _ => unreachable!(),
                })
            }

            GRAPH_RESET => Ok(GraphMessage::Reset),

            _ => Err(format!("Unknown graph message type: {}", graph_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HEAD_SIZE;

    #[test]
    fn test_graph_all() {
        let data = vec![0u8; 5 * 2 * std::mem::size_of::<f32>()];
        let graph_data = GraphsData {
            precision: Precision::F32,
            points: 5,
            data,
        };

        let mut head = [0u8; HEAD_SIZE];
        let message = GraphMessage::All(graph_data.clone());

        let data = message.write_message(&mut head[6..]);
        assert_eq!(data, Some(vec![0u8; 5 * 2 * std::mem::size_of::<f32>()]));

        let new_message = GraphMessage::read_message(&mut head[6..], data).unwrap();

        match new_message {
            GraphMessage::All(new_graph_data) => {
                assert_eq!(graph_data.data, new_graph_data.data);
                assert_eq!(graph_data.points, new_graph_data.points);
                assert_eq!(graph_data.precision, new_graph_data.precision);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_reset() {
        let mut head = [0u8; HEAD_SIZE];

        let message = GraphMessage::Reset;
        let data = message.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let message = GraphMessage::read_message(&mut head[6..], data).unwrap();

        match message {
            GraphMessage::Reset => (),
            _ => panic!("Wrong message type."),
        }
    }
}
