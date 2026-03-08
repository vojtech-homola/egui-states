use serde::{Deserialize, Serialize};

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

pub trait GraphElement: Clone + Copy + Send + Default + Sync + 'static {
    fn graph_type() -> GraphType;
    fn bytes_size() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl GraphElement for f32 {
    fn graph_type() -> GraphType {
        GraphType::F32
    }
}

impl GraphElement for f64 {
    fn graph_type() -> GraphType {
        GraphType::F64
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct GraphDataInfo {
    pub graph_type: GraphType,
    pub is_linear: bool,
    pub points: u64,
}

#[derive(Serialize, Deserialize)]
pub(crate) enum GraphHeader {
    Set(u16, GraphDataInfo),
    AddPoints(u16, GraphDataInfo),
    Remove(u16),
    Reset,
}
