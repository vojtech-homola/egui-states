use crate::transport::SIZE_START;

// graph ----------------------------------------------------------------------
/*
data:
| f32 | * count * lines
| f64 | * count * lines

---------
graph all:
| 1B - subtype | 1B - precision | 8B - u64 points | 8B - u64 lines | 8B - u64 data size |

---------
graph add points:
| 1B - subtype | 1B - precision | 8B - u64 points | 8B - u64 lines | 8B - u64 data size |

---------
graph add line:
| 1B - subtype | 1B - precision | 8B - u64 points | 8B - u64 lines | 8B - u64 data size |

---------
graph remove line:
| 1B - subtype | 8B - u64 index |

---------
graph reset:
| 1B - subtype |

*/
const GRAPH_F32: u8 = 60;
const GRAPH_F64: u8 = 61;

const GRAPH_ALL: u8 = 200;
const GRAPH_ADD_POINTS: u8 = 201;
const GRAPH_ADD_LINES: u8 = 202;
const GRAPH_REMOVE_LINE: u8 = 203;
const GRAPH_RESET: u8 = 204;

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
    pub lines: usize, // number of y lines -> +1 for x line
    pub data: Vec<u8>,
}

pub enum GraphMessage {
    All(GraphsData),
    AddPoints(GraphsData),
    AddLine(GraphsData),
    RemoveLine(usize),
    Reset,
}

impl GraphMessage {
    pub(crate) fn write_message(self, head: &mut [u8]) -> Option<Vec<u8>> {
        match self {
            GraphMessage::All(graph_data) => {
                head[0] = GRAPH_ALL;
                head[1] = graph_data.precision.to_u8();

                head[2..10].copy_from_slice(&(graph_data.points as u64).to_le_bytes());
                head[10..18].copy_from_slice(&(graph_data.lines as u64).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(graph_data.data.len() as u64).to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::AddPoints(graph_data) => {
                head[0] = GRAPH_ADD_POINTS;
                head[1] = graph_data.precision.to_u8();

                head[2..10].copy_from_slice(&(graph_data.points as u64).to_le_bytes());
                head[10..18].copy_from_slice(&(graph_data.lines as u64).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(graph_data.data.len() as u64).to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::AddLine(graph_data) => {
                head[0] = GRAPH_ADD_LINES;
                head[1] = graph_data.precision.to_u8();

                head[2..10].copy_from_slice(&(graph_data.points as u64).to_le_bytes());
                head[10..18].copy_from_slice(&(graph_data.lines as u64).to_le_bytes());
                head[SIZE_START..].copy_from_slice(&(graph_data.data.len() as u64).to_le_bytes());
                Some(graph_data.data)
            }
            GraphMessage::RemoveLine(index) => {
                head[0] = GRAPH_REMOVE_LINE;
                head[1..9].copy_from_slice(&(index as u64).to_le_bytes());
                None
            }
            GraphMessage::Reset => {
                head[0] = GRAPH_RESET;
                None
            }
        }
    }

    pub(crate) fn read_message(head: &mut [u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        let graph_type = head[0];

        match graph_type {
            GRAPH_ALL | GRAPH_ADD_LINES | GRAPH_ADD_POINTS => {
                let precision = match head[1] {
                    GRAPH_F32 => Precision::F32,
                    GRAPH_F64 => Precision::F64,
                    _ => return Err("Unknown precision".to_string()),
                };

                let points = u64::from_le_bytes(head[2..10].try_into().unwrap()) as usize;
                let lines = u64::from_le_bytes(head[10..18].try_into().unwrap()) as usize;

                let data = data.ok_or("Graph data is missing.".to_string())?;

                let transfer_lines = if graph_type == GRAPH_ADD_LINES {
                    lines
                } else {
                    lines + 1
                };
                if points * transfer_lines * precision.size() != data.len() {
                    return Err("Invalid data size for graph.".to_string());
                }

                let graphs_data = GraphsData {
                    precision,
                    points,
                    lines,
                    data,
                };

                Ok(match graph_type {
                    GRAPH_ALL => GraphMessage::All(graphs_data),
                    GRAPH_ADD_POINTS => GraphMessage::AddPoints(graphs_data),
                    GRAPH_ADD_LINES => GraphMessage::AddLine(graphs_data),
                    _ => unreachable!(),
                })
            }

            GRAPH_REMOVE_LINE => {
                let index = u64::from_le_bytes(head[1..9].try_into().unwrap()) as usize;
                Ok(GraphMessage::RemoveLine(index))
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
            lines: 1,
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
                assert_eq!(graph_data.lines, new_graph_data.lines);
                assert_eq!(graph_data.precision, new_graph_data.precision);
            }
            _ => panic!("Wrong message type."),
        }
    }

    #[test]
    fn test_remove_line() {
        let index = 5;
        let mut head = [0u8; HEAD_SIZE];

        let message = GraphMessage::RemoveLine(index);
        let data = message.write_message(&mut head[6..]);
        assert_eq!(data, None);

        let message = GraphMessage::read_message(&mut head[6..], data).unwrap();

        match message {
            GraphMessage::RemoveLine(new_index) => assert_eq!(index, new_index),
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
