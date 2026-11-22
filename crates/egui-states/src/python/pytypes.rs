use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyString};

use crate::typed::{ObjectType, VariantFields};

/// Generated bindings escape Python keywords (`from_`) and record the original
/// names in `__egui_names__`; type hashes must use the original names.
fn wire_name(owner: &Bound<PyAny>, name: String) -> PyResult<String> {
    let Some(names) = owner.getattr_opt("__egui_names__")? else {
        return Ok(name);
    };
    match names.cast::<PyDict>()?.get_item(&name)? {
        Some(wire) => wire.extract(),
        None => Ok(name),
    }
}

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
    /// A generated dataclass: its fields as (Python attribute name, type) in
    /// `__match_args__` order, and the class itself.
    Class(Vec<(Py<PyString>, PyObjectType)>, Py<PyAny>),
    List(u32, Box<PyObjectType>),
    Vec(Box<PyObjectType>),
    Map(Box<PyObjectType>, Box<PyObjectType>),
    Option(Box<PyObjectType>),
    Empty,
    /// A generated enum with data: the namespace class of the enum and its
    /// variants in declaration order, so a variant's index is its position on
    /// the wire.
    DataEnum(Py<PyAny>, Vec<PyVariant>),
}

/// One variant of a [`PyObjectType::DataEnum`]: a generated frozen dataclass,
/// whatever its kind.
pub(crate) struct PyVariant {
    /// The variant name on the wire.
    pub(crate) name: String,
    /// What the variant is in Rust; only the type hash needs it.
    kind: PyVariantKind,
    /// The fields as `(Python attribute, type)` in `__match_args__` order;
    /// none for a unit variant, `_0`, `_1`, ... for a tuple variant.
    pub(crate) fields: Vec<(Py<PyString>, PyObjectType)>,
    /// The variant class.
    pub(crate) class: Py<PyAny>,
}

#[derive(Clone, Copy)]
enum PyVariantKind {
    Unit,
    Named,
    Tuple,
}

impl PyVariant {
    fn clone_py(&self, py: Python) -> Self {
        Self {
            name: self.name.clone(),
            kind: self.kind,
            fields: self
                .fields
                .iter()
                .map(|(name, t)| (name.clone_ref(py), t.clone_py(py)))
                .collect(),
            class: self.class.clone_ref(py),
        }
    }

    /// Whether `obj` is a value of this variant.
    pub(crate) fn matches(&self, obj: &Bound<PyAny>) -> bool {
        obj.get_type().is(self.class.bind(obj.py()))
    }
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
            PyObjectType::Class(fields, py_obj) => {
                let cloned_fields = fields
                    .iter()
                    .map(|(name, t)| (name.clone_ref(py), t.clone_py(py)))
                    .collect();
                PyObjectType::Class(cloned_fields, py_obj.clone_ref(py))
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
            PyObjectType::DataEnum(class, variants) => PyObjectType::DataEnum(
                class.clone_ref(py),
                variants
                    .iter()
                    .map(|variant| variant.clone_py(py))
                    .collect(),
            ),
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
                        let name = wire_name(enum_type, name.extract::<String>()?)?;
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
            PyObjectType::Class(fields, obj) => {
                let struct_type = obj.bind(py);
                let name = struct_type.getattr("__name__")?.extract::<String>()?;
                let mut core_elements = Vec::with_capacity(fields.len());
                for (n, t) in fields {
                    let n = n.bind(py).to_str()?.to_string();
                    core_elements.push((wire_name(struct_type, n)?, t.get_core_type(py)?));
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
            PyObjectType::DataEnum(class, variants) => {
                let name = class.bind(py).getattr("__name__")?.extract::<String>()?;
                let mut core_variants = Vec::with_capacity(variants.len());
                for variant in variants {
                    let class = variant.class.bind(py);
                    let mut types = Vec::with_capacity(variant.fields.len());
                    for (field, t) in &variant.fields {
                        types.push((field.bind(py).to_str()?.to_string(), t.get_core_type(py)?));
                    }
                    let fields = match variant.kind {
                        PyVariantKind::Unit => VariantFields::Unit,
                        PyVariantKind::Named => VariantFields::Named(
                            types
                                .into_iter()
                                .map(|(field, t)| Ok((wire_name(class, field)?, t)))
                                .collect::<PyResult<_>>()?,
                        ),
                        PyVariantKind::Tuple => {
                            VariantFields::Tuple(types.into_iter().map(|(_, t)| t).collect())
                        }
                    };
                    core_variants.push((variant.name.clone(), fields));
                }
                ObjectType::DataEnum(name, core_variants)
            }
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

    // Fields are read by name, so the instance dict order does not matter.
    let names = class_type
        .bind(py)
        .getattr("__match_args__")?
        .extract::<Vec<String>>()?;
    if names.len() != object_types.len() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "{} has {} fields in __match_args__ but {} field types were given",
            class_type.bind(py),
            names.len(),
            object_types.len()
        )));
    }
    let fields = names
        .into_iter()
        .map(|name| PyString::intern(py, &name).unbind())
        .zip(object_types)
        .collect();

    Ok(PyObjectClass {
        object_type: PyObjectType::Class(fields, class_type),
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

/// A variant of an enum with data, for [`denu`].
#[pyclass(name = "PyVariant")]
pub(crate) struct PyVariantClass {
    variant: PyVariant,
}

/// Describes the variant `class_type`, whose dataclass fields have the types
/// `elements` in order.
fn variant(
    py: Python,
    name: String,
    kind: PyVariantKind,
    elements: &[Bound<PyObjectClass>],
    class_type: Py<PyAny>,
) -> PyResult<PyVariantClass> {
    let types: Vec<PyObjectType> = elements
        .iter()
        .map(|t| t.borrow().object_type.clone_py(py))
        .collect();
    let has_fields = !matches!(kind, PyVariantKind::Unit);
    if has_fields && (types.is_empty() || types.iter().any(|t| matches!(t, PyObjectType::Empty))) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Variant {name} needs at least one field and no Empty field"
        )));
    }
    // Fields are read by name, in the order of the dataclass fields.
    let names = class_type
        .bind(py)
        .getattr("__match_args__")?
        .extract::<Vec<String>>()?;
    if names.len() != types.len() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "{} has {} fields in __match_args__ but {} field types were given",
            class_type.bind(py),
            names.len(),
            types.len()
        )));
    }
    let fields = names
        .into_iter()
        .map(|name| PyString::intern(py, &name).unbind())
        .zip(types)
        .collect();
    Ok(PyVariantClass {
        variant: PyVariant {
            name,
            kind,
            fields,
            class: class_type,
        },
    })
}

#[pyfunction]
pub(crate) fn vunit(py: Python, name: String, class_type: Py<PyAny>) -> PyResult<PyVariantClass> {
    variant(py, name, PyVariantKind::Unit, &[], class_type)
}

#[pyfunction]
pub(crate) fn vnamed(
    py: Python,
    name: String,
    elements: Vec<Bound<PyObjectClass>>,
    class_type: Py<PyAny>,
) -> PyResult<PyVariantClass> {
    variant(py, name, PyVariantKind::Named, &elements, class_type)
}

#[pyfunction]
pub(crate) fn vtuple(
    py: Python,
    name: String,
    elements: Vec<Bound<PyObjectClass>>,
    class_type: Py<PyAny>,
) -> PyResult<PyVariantClass> {
    variant(py, name, PyVariantKind::Tuple, &elements, class_type)
}

#[pyfunction]
pub(crate) fn denu(
    py: Python,
    enum_type: Py<PyAny>,
    variants: Vec<Bound<PyVariantClass>>,
) -> PyResult<PyObjectClass> {
    if variants.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "{} needs at least one variant",
            enum_type.bind(py)
        )));
    }
    let variants = variants
        .iter()
        .map(|variant| variant.borrow().variant.clone_py(py))
        .collect();
    Ok(PyObjectClass {
        object_type: PyObjectType::DataEnum(enum_type, variants),
    })
}

#[pyfunction]
pub(crate) fn enu(enum_type: Bound<PyAny>) -> PyResult<PyObjectClass> {
    let obj = enum_type.unbind();

    Ok(PyObjectClass {
        object_type: PyObjectType::Enum(obj),
    })
}

#[cfg(test)]
mod tests {
    use pyo3::types::PyModule;

    use super::*;

    /// Generated bindings rename keywords (`True_`, `from_`). The type description
    /// feeds the type hash the client checks, so it must keep the Rust names.
    #[test]
    fn escaped_python_names_describe_their_wire_names() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::from_code(
                py,
                cr#"
from dataclasses import dataclass
from enum import IntEnum

class Keyword(IntEnum):
    None_ = 0
    Plain = 1
    __egui_names__ = {"None_": "None"}

@dataclass
class Reserved:
    from_: int
    plain: bool
    __egui_names__ = {"from_": "from"}
"#,
                c"escaped.py",
                c"escaped",
            )
            .unwrap();

            let keyword = PyObjectType::Enum(module.getattr("Keyword").unwrap().unbind());
            assert_eq!(
                keyword.get_core_type(py).unwrap(),
                ObjectType::Enum(
                    "Keyword".to_string(),
                    vec![("None".to_string(), 0), ("Plain".to_string(), 1)]
                )
            );

            let reserved = PyObjectType::Class(
                vec![
                    (PyString::new(py, "from_").unbind(), PyObjectType::U32),
                    (PyString::new(py, "plain").unbind(), PyObjectType::Bool),
                ],
                module.getattr("Reserved").unwrap().unbind(),
            );
            assert_eq!(
                reserved.get_core_type(py).unwrap(),
                ObjectType::Struct(
                    "Reserved".to_string(),
                    vec![
                        ("from".to_string(), ObjectType::U32),
                        ("plain".to_string(), ObjectType::Bool)
                    ]
                )
            );
        });
    }
}
