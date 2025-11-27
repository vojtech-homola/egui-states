use pyo3::prelude::*;

use egui_states_core::types::ObjectType;

#[pyclass]
pub(crate) struct PyObjectType {
    pub object_type: ObjectType,
}

#[pymethods]
impl PyObjectType {
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
    fn empty() -> Self {
        Self {
            object_type: ObjectType::Empty,
        }
    }

    #[staticmethod]
    #[pyo3(signature = (elements, optional=false))]
    fn tuple_(elements: Vec<Bound<PyObjectType>>, optional: bool) -> PyResult<Self> {
        let object_types: Vec<ObjectType> = elements
            .iter()
            .map(|t| t.borrow().object_type.clone())
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
    #[pyo3(signature = (element_type, size, optional=false))]
    fn list_(element_type: Bound<PyObjectType>, size: u32, optional: bool) -> PyResult<Self> {
        let elem_type = element_type.borrow().object_type.clone();
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
    fn vec(element_type: Bound<PyObjectType>, optional: bool) -> PyResult<Self> {
        let val_type = element_type.borrow().object_type.clone();

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
        key_type: Bound<PyObjectType>,
        value_type: Bound<PyObjectType>,
        optional: bool,
    ) -> PyResult<Self> {
        let k_type = key_type.borrow().object_type.clone();
        let v_type = value_type.borrow().object_type.clone();

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
