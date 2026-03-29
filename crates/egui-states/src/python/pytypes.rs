use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::transport::ObjectType;

pub(crate) enum PyObjectType {
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
    Tuple(Vec<PyObjectType>),
    Class(Vec<PyObjectType>, Py<PyAny>),
    List(u32, Box<PyObjectType>),
    Vec(Box<PyObjectType>),
    Map(Box<PyObjectType>, Box<PyObjectType>),
    Option(Box<PyObjectType>),
    Empty,
}

impl PyObjectType {
    pub(crate) fn clone_py(&self, py: Python) -> Self {
        match self {
            PyObjectType::Enum(py_enum) => PyObjectType::Enum(py_enum.clone_ref(py)),
            PyObjectType::U8 => PyObjectType::U8,
            PyObjectType::U16 => PyObjectType::U16,
            PyObjectType::U32 => PyObjectType::U32,
            PyObjectType::U64 => PyObjectType::U64,
            PyObjectType::I8 => PyObjectType::I8,
            PyObjectType::I16 => PyObjectType::I16,
            PyObjectType::I32 => PyObjectType::I32,
            PyObjectType::I64 => PyObjectType::I64,
            PyObjectType::F32 => PyObjectType::F32,
            PyObjectType::F64 => PyObjectType::F64,
            PyObjectType::String => PyObjectType::String,
            PyObjectType::Bool => PyObjectType::Bool,
            PyObjectType::Tuple(vec) => {
                let cloned_vec = vec.iter().map(|t| t.clone_py(py)).collect();
                PyObjectType::Tuple(cloned_vec)
            }
            PyObjectType::Class(vec, py_obj) => {
                let cloned_vec = vec.iter().map(|t| t.clone_py(py)).collect();
                PyObjectType::Class(cloned_vec, py_obj.clone_ref(py))
            }
            PyObjectType::List(size, elem_type) => {
                PyObjectType::List(*size, Box::new(elem_type.clone_py(py)))
            }
            PyObjectType::Vec(elem_type) => PyObjectType::Vec(Box::new(elem_type.clone_py(py))),
            PyObjectType::Map(key_type, value_type) => PyObjectType::Map(
                Box::new(key_type.clone_py(py)),
                Box::new(value_type.clone_py(py)),
            ),
            PyObjectType::Option(inner_type) => {
                PyObjectType::Option(Box::new(inner_type.clone_py(py)))
            }
            PyObjectType::Empty => PyObjectType::Empty,
        }
    }

    pub(crate) fn get_core_type(&self, py: Python) -> PyResult<ObjectType> {
        let obj = match self {
            PyObjectType::U8 => ObjectType::U8,
            PyObjectType::U16 => ObjectType::U16,
            PyObjectType::U32 => ObjectType::U32,
            PyObjectType::U64 => ObjectType::U64,
            PyObjectType::I8 => ObjectType::I8,
            PyObjectType::I16 => ObjectType::I16,
            PyObjectType::I32 => ObjectType::I32,
            PyObjectType::I64 => ObjectType::I64,
            PyObjectType::F32 => ObjectType::F32,
            PyObjectType::F64 => ObjectType::F64,
            PyObjectType::String => ObjectType::String,
            PyObjectType::Bool => ObjectType::Bool,
            PyObjectType::Enum(obj) => {
                let enum_type = obj.bind(py);
                let member_map = enum_type.getattr("_member_map_")?;
                let members = member_map
                    .cast::<PyDict>()?
                    .iter()
                    .map(|(name, value)| {
                        let name = name.extract::<String>()?;
                        let value = value.getattr("value")?.extract::<i32>()?;
                        Ok((name, value))
                    })
                    .collect::<PyResult<Vec<(String, i32)>>>()?;

                let name = enum_type.getattr("__name__")?.extract::<String>()?;
                ObjectType::Enum(name, members)
            }
            PyObjectType::Tuple(elements) => {
                let mut core_elements = Vec::with_capacity(elements.len());
                for t in elements {
                    core_elements.push(t.get_core_type(py)?);
                }
                ObjectType::Tuple(core_elements)
            }
            PyObjectType::Class(elements, obj) => {
                let struct_type = obj.bind(py);
                let name = struct_type.getattr("__name__")?.extract::<String>()?;
                let memebers = struct_type
                    .getattr("__match_args__")?
                    .extract::<Vec<String>>()?;
                let mut core_elements = Vec::with_capacity(elements.len());
                for (n, t) in memebers.iter().zip(elements.iter()) {
                    core_elements.push((n.clone(), t.get_core_type(py)?));
                }

                ObjectType::Struct(name, core_elements)
            }
            PyObjectType::List(size, elem_type) => {
                ObjectType::List(*size, Box::new(elem_type.get_core_type(py)?))
            }
            PyObjectType::Vec(elem_type) => ObjectType::Vec(Box::new(elem_type.get_core_type(py)?)),
            PyObjectType::Map(key_type, value_type) => ObjectType::Map(
                Box::new(key_type.get_core_type(py)?),
                Box::new(value_type.get_core_type(py)?),
            ),
            PyObjectType::Option(inner_type) => {
                ObjectType::Option(Box::new(inner_type.get_core_type(py)?))
            }
            PyObjectType::Empty => ObjectType::Empty,
        };

        Ok(obj)
    }

    #[inline]
    pub(crate) fn get_hash(&self, py: Python) -> PyResult<u32> {
        let res = self.get_core_type(py)?.get_hash();
        Ok(res)
    }
}

#[pyclass(name = "PyObjectType")]
pub(crate) struct PyObjectClass {
    pub object_type: PyObjectType,
}

pub(crate) const U8: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::U8,
};

pub(crate) const U16: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::U16,
};

pub(crate) const U32: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::U32,
};

pub(crate) const U64: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::U64,
};

pub(crate) const I8: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::I8,
};

pub(crate) const I16: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::I16,
};

pub(crate) const I32: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::I32,
};

pub(crate) const I64: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::I64,
};

pub(crate) const F32: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::F32,
};

pub(crate) const F64: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::F64,
};

pub(crate) const BO: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::Bool,
};

pub(crate) const STR: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::String,
};

pub(crate) const EMP: PyObjectClass = PyObjectClass {
    object_type: PyObjectType::Empty,
};

#[pyfunction]
pub(crate) fn opt(py: Python, pytype: &Bound<PyObjectClass>) -> PyObjectClass {
    let object_type = PyObjectType::Option(Box::new(pytype.borrow().object_type.clone_py(py)));

    PyObjectClass { object_type }
}

#[pyfunction]
pub(crate) fn tu(py: Python, elements: Vec<Bound<PyObjectClass>>) -> PyResult<PyObjectClass> {
    let object_types: Vec<PyObjectType> = elements
        .iter()
        .map(|t| t.borrow().object_type.clone_py(py))
        .collect();

    if object_types
        .iter()
        .any(|t| matches!(t, PyObjectType::Empty))
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Tuple cannot contain Empty type",
        ));
    }

    Ok(PyObjectClass {
        object_type: PyObjectType::Tuple(object_types),
    })
}

#[pyfunction]
pub(crate) fn cl(
    py: Python,
    elements: Vec<Bound<PyObjectClass>>,
    class_type: Py<PyAny>,
) -> PyResult<PyObjectClass> {
    let object_types: Vec<PyObjectType> = elements
        .iter()
        .map(|t| t.borrow().object_type.clone_py(py))
        .collect();

    if object_types
        .iter()
        .any(|t| matches!(t, PyObjectType::Empty))
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Tuple cannot contain Empty type",
        ));
    }

    Ok(PyObjectClass {
        object_type: PyObjectType::Class(object_types, class_type),
    })
}

#[pyfunction]
pub(crate) fn li(
    py: Python,
    element_type: Bound<PyObjectClass>,
    size: u32,
) -> PyResult<PyObjectClass> {
    let elem_type = element_type.borrow().object_type.clone_py(py);
    if matches!(elem_type, PyObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "List cannot contain Empty type",
        ));
    }

    Ok(PyObjectClass {
        object_type: PyObjectType::List(size, Box::new(elem_type)),
    })
}

#[pyfunction]
pub(crate) fn vec(py: Python, element_type: Bound<PyObjectClass>) -> PyResult<PyObjectClass> {
    let val_type = element_type.borrow().object_type.clone_py(py);

    if matches!(val_type, PyObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Vec cannot contain Empty type",
        ));
    }

    Ok(PyObjectClass {
        object_type: PyObjectType::Vec(Box::new(val_type)),
    })
}

#[pyfunction]
pub(crate) fn map(
    py: Python,
    key_type: &Bound<PyObjectClass>,
    value_type: &Bound<PyObjectClass>,
) -> PyResult<PyObjectClass> {
    let k_type = key_type.borrow().object_type.clone_py(py);
    let v_type = value_type.borrow().object_type.clone_py(py);

    if matches!(k_type, PyObjectType::Empty) || matches!(v_type, PyObjectType::Empty) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Map cannot contain Empty type",
        ));
    }

    Ok(PyObjectClass {
        object_type: PyObjectType::Map(Box::new(k_type), Box::new(v_type)),
    })
}

#[pyfunction]
pub(crate) fn enu(enum_type: Bound<PyAny>) -> PyResult<PyObjectClass> {
    let obj = enum_type.unbind();

    Ok(PyObjectClass {
        object_type: PyObjectType::Enum(obj),
    })
}
