use pyo3::buffer::PyBuffer;
use pyo3::prelude::*;
use pyo3::exceptions::PyValueError;

use egui_states_core::graphs::GraphElement;

use crate::graphs::{GraphData, ValueGraphs};

pub(crate) fn set_graph<T: GraphElement>(
    idx: u16,
    buffer: &PyBuffer<T>,
    value: &ValueGraphs,
    update: bool,
) -> PyResult<()> {
    let shape = buffer.shape();
    let stride = buffer.strides().last().ok_or(PyValueError::new_err(
        "Graph data must have at least 1 dimension.",
    ))?;
    if *stride != size_of::<T>() as isize {
        return Err(PyValueError::new_err(
            "Graph line data must have a contiguous memory layout.",
        ));
    }

    if shape.len() == 1 {

    } else if shape.len() == 2 {
        if shape[0] < 2 {
            return Err(PyValueError::new_err(
                "Graph data must have at least 2 points.",
            ));
        }

        let points = shape[0];
        
    } else {
        return Err(PyValueError::new_err(
            "Graph data must have 1 or 2 dimensions.",
        ));
    }

    let graph_data = GraphData {
        graph_type: T::graph_type(),
        y,
        x,
        size,
    };

    value.set(idx, graph_data, update);
    Ok(())
}
