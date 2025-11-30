use std::ptr::copy_nonoverlapping;
use std::slice::from_raw_parts;

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};

use egui_states_core::graphs::{GraphElement, GraphTyped};

use crate::graphs::GraphData;

pub(crate) fn buffer_to_data<T: GraphElement>(buffer: &PyBuffer<T>) -> PyResult<GraphData> {
    let shape = buffer.shape();
    let stride = buffer.strides().last().ok_or(PyValueError::new_err(
        "Graph data must have at least 1 dimension.",
    ))?;
    if *stride != size_of::<T>() as isize {
        return Err(PyValueError::new_err(
            "Graph line data must have a contiguous memory layout.",
        ));
    }

    let graph_data = if shape.len() == 1 {
        if shape[0] < 2 {
            return Err(PyValueError::new_err(
                "Graph data must have at least 2 points.",
            ));
        }

        GraphData {
            graph_type: T::graph_type(),
            y: buffer.get_ptr(&[0]) as *const u8,
            x: None,
            size: shape[0] * T::bytes_size(),
        }
    } else if shape.len() == 2 {
        if shape[0] != 2 {
            return Err(PyValueError::new_err(
                "Graph data must have 2 lines (x, y).",
            ));
        }
        if shape[1] < 2 {
            return Err(PyValueError::new_err(
                "Graph data must have at least 2 points.",
            ));
        }

        GraphData {
            graph_type: T::graph_type(),
            y: buffer.get_ptr(&[0, 0]) as *const u8,
            x: Some(buffer.get_ptr(&[1, 0]) as *const u8),
            size: shape[1] * T::bytes_size(),
        }
    } else {
        return Err(PyValueError::new_err(
            "Graph data must have 1 or 2 dimensions.",
        ));
    };

    Ok(graph_data)
}

pub(crate) fn graph_to_buffer<'py, T: GraphElement>(
    py: Python<'py>,
    graph: &GraphTyped,
) -> PyResult<Bound<'py, PyTuple>> {
    match graph.x {
        Some(ref x) => {
            let size = (x.len() + graph.y.len()) * size_of::<T>();
            let bytes = PyBytes::new_with(py, size, |buf| {
                let mut ptr = buf.as_mut_ptr();
                unsafe {
                    copy_nonoverlapping(x.as_ptr(), ptr, x.len());
                    ptr = ptr.add(x.len());
                    copy_nonoverlapping(graph.y.as_ptr(), ptr, graph.y.len());
                };
                Ok(())
            })?;

            let shape = (2usize, graph.y.len(), size_of::<T>());
            (bytes, shape).into_pyobject(py)
        }
        None => {
            let size = graph.y.len() * size_of::<T>();
            let data = unsafe { from_raw_parts(graph.y.as_ptr(), size) };
            let bytes = PyBytes::new(py, data);
            (bytes, (graph.y.len(), size_of::<T>())).into_pyobject(py)
        }
    }
}
