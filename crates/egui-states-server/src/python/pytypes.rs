use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use egui_states_core::types::ObjectType as CoreObjectType;

pub(crate) enum ObjectType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F64,
    F32,
    String,
    Bool,
    Enum(Py<PyAny>),
    Tuple(Vec<ObjectType>),
    Class(Vec<ObjectType>, Py<PyAny>),
    List(u32, Box<ObjectType>),
    Vec(Box<ObjectType>),
    Map(Box<ObjectType>, Box<ObjectType>),
    Option(Box<ObjectType>),
    Empty,
}

impl ObjectType {
    pub(crate) fn clone_py(&self, py: Python) -> Self {
        match self {
            ObjectType::Enum(py_enum) => ObjectType::Enum(py_enum.clone_ref(py)),
            ObjectType::U8 => ObjectType::U8,
            ObjectType::U16 => ObjectType::U16,
            ObjectType::U32 => ObjectType::U32,
            ObjectType::U64 => ObjectType::U64,
            ObjectType::I8 => ObjectType::I8,
            ObjectType::I16 => ObjectType::I16,
            ObjectType::I32 => ObjectType::I32,
            ObjectType::I64 => ObjectType::I64,
            ObjectType::F32 => ObjectType::F32,
            ObjectType::F64 => ObjectType::F64,
            ObjectType::String => ObjectType::String,
            ObjectType::Bool => ObjectType::Bool,
            ObjectType::Tuple(vec) => {
                let cloned_vec = vec.iter().map(|t| t.clone_py(py)).collect();
                ObjectType::Tuple(cloned_vec)
            }
            ObjectType::Class(vec, py_obj) => {
                let cloned_vec = vec.iter().map(|t| t.clone_py(py)).collect();
                ObjectType::Class(cloned_vec, py_obj.clone_ref(py))
            }
            ObjectType::List(size, elem_type) => {
                ObjectType::List(*size, Box::new(elem_type.clone_py(py)))
            }
            ObjectType::Vec(elem_type) => ObjectType::Vec(Box::new(elem_type.clone_py(py))),
            ObjectType::Map(key_type, value_type) => ObjectType::Map(
                Box::new(key_type.clone_py(py)),
                Box::new(value_type.clone_py(py)),
            ),
            ObjectType::Option(inner_type) => ObjectType::Option(Box::new(inner_type.clone_py(py))),
            ObjectType::Empty => ObjectType::Empty,
        }
    }

    fn get_core_type(&self, py: Python) -> PyResult<CoreObjectType> {
        let obj = match self {
            ObjectType::U8 => CoreObjectType::U8,
            ObjectType::U16 => CoreObjectType::U16,
            ObjectType::U32 => CoreObjectType::U32,
            ObjectType::U64 => CoreObjectType::U64,
            ObjectType::I8 => CoreObjectType::I8,
            ObjectType::I16 => CoreObjectType::I16,
            ObjectType::I32 => CoreObjectType::I32,
            ObjectType::I64 => CoreObjectType::I64,
            ObjectType::F32 => CoreObjectType::F32,
            ObjectType::F64 => CoreObjectType::F64,
            ObjectType::String => CoreObjectType::String,
            ObjectType::Bool => CoreObjectType::Bool,
            ObjectType::Enum(obj) => {
                let enum_type = obj.bind(py);
                // let members = enum_type
                //     .call_method0("_get_members")?
                //     .extract::<Vec<(String, i64)>>()?;
                let member_map = enum_type.getattr("_member_map_")?;
                let members = member_map
                    .cast::<PyDict>()?
                    .iter()
                    .map(|(name, value)| {
                        let name = name.extract::<String>()?;
                        let value = value.getattr("value")?.extract::<i64>()?;
                        Ok((name, value))
                    })
                    .collect::<PyResult<Vec<(String, i64)>>>()?;

                let name = enum_type.getattr("__name__")?.extract::<String>()?;
                CoreObjectType::Enum(name, members)
            }
            ObjectType::Tuple(elements) => {
                let mut core_elements = Vec::with_capacity(elements.len());
                for t in elements {
                    core_elements.push(t.get_core_type(py)?);
                }
                CoreObjectType::Tuple(core_elements)
            }
            ObjectType::Class(elements, obj) => {
                let struct_type = obj.bind(py);
                let name = struct_type.getattr("__name__")?.extract::<String>()?;
                let memebers = struct_type
                    .getattr("__match_args__")?
                    .extract::<Vec<String>>()?;
                let mut core_elements = Vec::with_capacity(elements.len());
                for (n, t) in memebers.iter().zip(elements.iter()) {
                    core_elements.push((n.clone(), t.get_core_type(py)?));
                }

                CoreObjectType::Struct(name, core_elements)
            }
            ObjectType::List(size, elem_type) => {
                CoreObjectType::List(*size, Box::new(elem_type.get_core_type(py)?))
            }
            ObjectType::Vec(elem_type) => {
                CoreObjectType::Vec(Box::new(elem_type.get_core_type(py)?))
            }
            ObjectType::Map(key_type, value_type) => CoreObjectType::Map(
                Box::new(key_type.get_core_type(py)?),
                Box::new(value_type.get_core_type(py)?),
            ),
            ObjectType::Option(inner_type) => {
                CoreObjectType::Option(Box::new(inner_type.get_core_type(py)?))
            }
            ObjectType::Empty => CoreObjectType::Empty,
        };

        Ok(obj)
    }

    pub(crate) fn get_hash(&self, py: Python) -> PyResult<u64> {
        let res = self.get_core_type(py)?.get_hash();
        Ok(res)
    }
}

#[pyclass]
pub(crate) struct PyObjectType {
    pub object_type: ObjectType,
}

pub(crate) const U8: PyObjectType = PyObjectType {
    object_type: ObjectType::U8,
};

pub(crate) const U16: PyObjectType = PyObjectType {
    object_type: ObjectType::U16,
};

pub(crate) const U32: PyObjectType = PyObjectType {
    object_type: ObjectType::U32,
};

pub(crate) const U64: PyObjectType = PyObjectType {
    object_type: ObjectType::U64,
};

pub(crate) const I8: PyObjectType = PyObjectType {
    object_type: ObjectType::I8,
};

pub(crate) const I16: PyObjectType = PyObjectType {
    object_type: ObjectType::I16,
};

pub(crate) const I32: PyObjectType = PyObjectType {
    object_type: ObjectType::I32,
};

pub(crate) const I64: PyObjectType = PyObjectType {
    object_type: ObjectType::I64,
};

pub(crate) const F32: PyObjectType = PyObjectType {
    object_type: ObjectType::F32,
};

pub(crate) const F64: PyObjectType = PyObjectType {
    object_type: ObjectType::F64,
};

pub(crate) const BO: PyObjectType = PyObjectType {
    object_type: ObjectType::Bool,
};

pub(crate) const STR: PyObjectType = PyObjectType {
    object_type: ObjectType::String,
};

pub(crate) const EMP: PyObjectType = PyObjectType {
    object_type: ObjectType::Empty,
};

#[pyfunction]
pub(crate) fn opt(py: Python, pytype: &Bound<PyObjectType>) -> PyObjectType {
    let object_type = ObjectType::Option(Box::new(pytype.borrow().object_type.clone_py(py)));

    PyObjectType { object_type }
}

#[pyfunction]
pub(crate) fn tu(py: Python, elements: Vec<Bound<PyObjectType>>) -> PyResult<PyObjectType> {
    let object_types: Vec<ObjectType> = elements
        .iter()
        .map(|t| t.borrow().object_type.clone_py(py))
        .collect();

    if object_types.iter().any(|t| matches!(t, ObjectType::Empty)) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Tuple cannot contain Empty type",
        ));
    }

    Ok(PyObjectType {
        object_type: ObjectType::Tuple(object_types),
    })
}

#[pyfunction]
pub(crate) fn cl(
    py: Python,
    elements: Vec<Bound<PyObjectType>>,
    class_type: Py<PyAny>,
) -> PyResult<PyObjectType> {
    let object_types: Vec<ObjectType> = elements
        .iter()
        .map(|t| t.borrow().object_type.clone_py(py))
        .collect();

    if object_types.iter().any(|t| matches!(t, ObjectType::Empty)) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Tuple cannot contain Empty type",
        ));
    }

    Ok(PyObjectType {
        object_type: ObjectType::Class(object_types, class_type),
    })
}

#[pyfunction]
pub(crate) fn li(
    py: Python,
    element_type: Bound<PyObjectType>,
    size: u32,
) -> PyResult<PyObjectType> {
    let elem_type = element_type.borrow().object_type.clone_py(py);
    if matches!(elem_type, ObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "List cannot contain Empty type",
        ));
    }

    Ok(PyObjectType {
        object_type: ObjectType::List(size, Box::new(elem_type)),
    })
}

#[pyfunction]
pub(crate) fn vec(py: Python, element_type: Bound<PyObjectType>) -> PyResult<PyObjectType> {
    let val_type = element_type.borrow().object_type.clone_py(py);

    if matches!(val_type, ObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Vec cannot contain Empty type",
        ));
    }

    Ok(PyObjectType {
        object_type: ObjectType::Vec(Box::new(val_type)),
    })
}

#[pyfunction]
pub(crate) fn map(
    py: Python,
    key_type: &Bound<PyObjectType>,
    value_type: &Bound<PyObjectType>,
) -> PyResult<PyObjectType> {
    let k_type = key_type.borrow().object_type.clone_py(py);
    let v_type = value_type.borrow().object_type.clone_py(py);

    if matches!(k_type, ObjectType::Empty) || matches!(v_type, ObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Map cannot contain Empty type",
        ));
    }

    Ok(PyObjectType {
        object_type: ObjectType::Map(Box::new(k_type), Box::new(v_type)),
    })
}

#[pyfunction]
pub(crate) fn enu(enum_type: Bound<PyAny>) -> PyResult<PyObjectType> {
    let obj = enum_type.unbind();

    Ok(PyObjectType {
        object_type: ObjectType::Enum(obj),
    })
}
