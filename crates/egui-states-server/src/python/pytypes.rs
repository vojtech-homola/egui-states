use pyo3::prelude::*;

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
    Enum(Py<PyAny>, u32),
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
            ObjectType::Enum(py_enum, max) => ObjectType::Enum(py_enum.clone_ref(py), *max),
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

    fn get_core_type(&self) -> CoreObjectType {
        match self {
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
            ObjectType::Enum(_, max) => CoreObjectType::Enum(*max),
            ObjectType::Tuple(elements) => {
                let core_elements = elements.iter().map(|t| t.get_core_type()).collect();
                CoreObjectType::Tuple(core_elements)
            }
            ObjectType::Class(elements, _) => {
                let core_elements = elements.iter().map(|t| t.get_core_type()).collect();
                CoreObjectType::Tuple(core_elements)
            }
            ObjectType::List(size, elem_type) => {
                CoreObjectType::List(*size, Box::new(elem_type.get_core_type()))
            }
            ObjectType::Vec(elem_type) => CoreObjectType::Vec(Box::new(elem_type.get_core_type())),
            ObjectType::Map(key_type, value_type) => CoreObjectType::Map(
                Box::new(key_type.get_core_type()),
                Box::new(value_type.get_core_type()),
            ),
            ObjectType::Option(inner_type) => {
                CoreObjectType::Option(Box::new(inner_type.get_core_type()))
            }
            ObjectType::Empty => panic!("Empty type has no core representation"),
        }
    }

    pub(crate) fn get_hash(&self) -> u64 {
        self.get_core_type().get_hash()
    }
}

#[pyclass]
pub(crate) struct PyObjectType {
    pub object_type: ObjectType,
}

#[pymethods]
impl PyObjectType {
    #[staticmethod]
    fn optional(py: Python, pytype: &Bound<PyObjectType>) -> Self {
        let object_type = ObjectType::Option(Box::new(pytype.borrow().object_type.clone_py(py)));

        Self { object_type }
    }
    
    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn u8(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::U8)),
            false => ObjectType::U8,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn u16(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::U16)),
            false => ObjectType::U16,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn u32(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::U32)),
            false => ObjectType::U32,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn u64(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::U64)),
            false => ObjectType::U64,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn i8(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::I8)),
            false => ObjectType::I8,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn i16(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::I16)),
            false => ObjectType::I16,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn i32(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::I32)),
            false => ObjectType::I32,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn i64(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::I64)),
            false => ObjectType::I64,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn f32(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::F32)),
            false => ObjectType::F32,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn f64(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::F64)),
            false => ObjectType::F64,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn boolean(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::Bool)),
            false => ObjectType::Bool,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (optional=false))]
    fn string(optional: bool) -> Self {
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::String)),
            false => ObjectType::String,
        };

        Self { object_type }
    }

    #[staticmethod]
    #[pyo3(signature = (enum_obj, optional=false))]
    fn enum_(enum_obj: Bound<PyAny>, optional: bool) -> PyResult<Self> {
        let len = enum_obj.len()? as u32;
        let obj = enum_obj.unbind();
        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::Enum(obj, len))),
            false => ObjectType::Enum(obj, len),
        };

        Ok(Self { object_type })
    }

    #[staticmethod]
    fn empty() -> Self {
        Self {
            object_type: ObjectType::Empty,
        }
    }

    #[staticmethod]
    #[pyo3(signature = (elements, optional=false))]
    fn tuple_(py: Python, elements: Vec<Bound<PyObjectType>>, optional: bool) -> PyResult<Self> {
        let object_types: Vec<ObjectType> = elements
            .iter()
            .map(|t| t.borrow().object_type.clone_py(py))
            .collect();

        if object_types.iter().any(|t| matches!(t, ObjectType::Empty)) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Tuple cannot contain Empty type",
            ));
        }

        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::Tuple(object_types))),
            false => ObjectType::Tuple(object_types),
        };

        Ok(Self { object_type })
    }

    #[staticmethod]
    #[pyo3(signature = (elements, class_type, optional=false))]
    fn class_(
        py: Python,
        elements: Vec<Bound<PyObjectType>>,
        class_type: Py<PyAny>,
        optional: bool,
    ) -> PyResult<Self> {
        let object_types: Vec<ObjectType> = elements
            .iter()
            .map(|t| t.borrow().object_type.clone_py(py))
            .collect();

        if object_types.iter().any(|t| matches!(t, ObjectType::Empty)) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Tuple cannot contain Empty type",
            ));
        }

        let object_type = match optional {
            true => ObjectType::Option(Box::new(ObjectType::Class(object_types, class_type))),
            false => ObjectType::Class(object_types, class_type),
        };

        Ok(Self { object_type })
    }

    #[staticmethod]
    #[pyo3(signature = (element_type, size, optional=false))]
    fn list_(
        py: Python,
        element_type: Bound<PyObjectType>,
        size: u32,
        optional: bool,
    ) -> PyResult<Self> {
        let elem_type = element_type.borrow().object_type.clone_py(py);
        if matches!(elem_type, ObjectType::Empty) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "List cannot contain Empty type",
            ));
        }

        let elem_type = ObjectType::List(size, Box::new(elem_type));

        let object_type = match optional {
            true => ObjectType::Option(Box::new(elem_type)),
            false => elem_type,
        };

        Ok(Self { object_type })
    }

    #[staticmethod]
    #[pyo3(signature = (element_type, optional=false))]
    fn vec(py: Python, element_type: Bound<PyObjectType>, optional: bool) -> PyResult<Self> {
        let val_type = element_type.borrow().object_type.clone_py(py);

        if matches!(val_type, ObjectType::Empty) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Vec cannot contain Empty type",
            ));
        }

        let val_type = ObjectType::Vec(Box::new(val_type));

        let object_type = match optional {
            true => ObjectType::Option(Box::new(val_type)),
            false => val_type,
        };

        Ok(Self { object_type })
    }

    #[staticmethod]
    #[pyo3(signature = (key_type, value_type, optional=false))]
    fn map(
        py: Python,
        key_type: Bound<PyObjectType>,
        value_type: Bound<PyObjectType>,
        optional: bool,
    ) -> PyResult<Self> {
        let k_type = key_type.borrow().object_type.clone_py(py);
        let v_type = value_type.borrow().object_type.clone_py(py);

        if matches!(k_type, ObjectType::Empty) || matches!(v_type, ObjectType::Empty) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Map cannot contain Empty type",
            ));
        }

        let map_type = ObjectType::Map(Box::new(k_type), Box::new(v_type));

        let object_type = match optional {
            true => ObjectType::Option(Box::new(map_type)),
            false => map_type,
        };

        Ok(Self { object_type })
    }
}
