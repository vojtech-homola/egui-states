use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::types::{PyDict, PyList, PyNone, PySequence, PySequenceMethods, PyTuple};
use pyo3::{IntoPyObjectExt, prelude::*};

use crate::python::pytypes::{PyObjectType, PyVariant};
use crate::python::value_parsing::{ValueCreator, ValueParser};

/// The error for `obj` not being a value of the enum `class`. Passing a
/// variant class instead of a value, as in `Command.Stop` for `Command.Stop()`,
/// gets its own message.
fn not_a_variant(
    class: &Bound<PyAny>,
    variants: &[PyVariant],
    obj: &Bound<PyAny>,
) -> PyResult<PyErr> {
    let enum_name = class.getattr("__qualname__")?;
    if let Some(variant) = variants.iter().find(|v| obj.is(v.class.bind(obj.py()))) {
        let name = obj.getattr("__qualname__")?;
        let call = match variant.fields.len() {
            0 => format!("{name}()"),
            _ => format!("{name}(...)"),
        };
        return Ok(PyTypeError::new_err(format!(
            "Expected a value of {enum_name}, got the class {name}; create a value with {call}"
        )));
    }
    Ok(PyTypeError::new_err(format!(
        "Expected a variant of {enum_name}, got {}",
        obj.repr()?
    )))
}

/// One step from a value to the part of it that failed to convert.
enum Step {
    /// The class of a struct or an enum with data; shown only first.
    Type(String),
    /// A struct field, a variant or a variant field.
    Attribute(String),
    /// An item of a tuple, a fixed list or a list.
    Index(usize),
    /// The value of a map under the key with this `repr`.
    Value(String),
    /// The map key with this `repr`.
    Key(String),
}

/// Where a value passed to [`serialize_py_in`] sits in its collection.
pub(crate) enum Place<'a, 'py> {
    /// An item of a list.
    Index(usize),
    /// The value of a map under this key.
    Value(&'a Bound<'py, PyAny>),
    /// A key of a map.
    Key,
}

/// A conversion error and the steps to the part that caused it, innermost
/// first. The steps are collected only while the error returns, so a value
/// that converts pays nothing for them.
struct Failure {
    error: PyErr,
    steps: Vec<Step>,
}

impl<E: Into<PyErr>> From<E> for Failure {
    fn from(error: E) -> Self {
        Self {
            error: error.into(),
            steps: Vec::new(),
        }
    }
}

impl Failure {
    fn inside(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        self.steps.extend(steps);
        self
    }

    /// The error with the path in front of its message, as in
    /// `Command.SetGain._0: must be real number, not str`.
    fn into_error(self, py: Python) -> PyErr {
        if self.steps.is_empty() {
            return self.error;
        }
        let mut path = String::new();
        for step in self.steps.iter().rev() {
            match step {
                Step::Type(name) if path.is_empty() => path.push_str(name),
                Step::Type(_) => {}
                Step::Attribute(name) if path.is_empty() => path.push_str(name),
                Step::Attribute(name) => {
                    path.push('.');
                    path.push_str(name);
                }
                Step::Index(index) => path.push_str(&format!("[{index}]")),
                Step::Value(key) => path.push_str(&format!("[{key}]")),
                Step::Key(key) => path.push_str(&format!("[key {key}]")),
            }
        }

        // The exception keeps its type. One whose arguments are not a single
        // message, as `UnicodeEncodeError`, gets the path as a note instead.
        let value = self.error.value(py);
        let message = value
            .getattr("args")
            .and_then(|args| args.extract::<(String,)>());
        let _ = match message {
            Ok((message,)) => value.setattr("args", (format!("{path}: {message}"),)),
            Err(_) => value
                .call_method1("add_note", (format!("In {path}"),))
                .map(drop),
        };
        self.error
    }
}

/// A `repr` short enough for an error message.
fn short_repr(obj: &Bound<PyAny>) -> String {
    const LIMIT: usize = 60;
    let repr = match obj.repr() {
        Ok(repr) => repr.to_string(),
        Err(_) => return "<object>".to_string(),
    };
    match repr.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}...", &repr[..end]),
        None => repr,
    }
}

/// The name of a class in an error message.
fn class_name(class: &Bound<PyAny>, attribute: &str) -> String {
    match class.getattr(attribute) {
        Ok(name) => name.to_string(),
        Err(_) => class.to_string(),
    }
}

fn add<T: serde::Serialize>(creator: &mut ValueCreator, value: &T) -> PyResult<()> {
    creator
        .add(value)
        .map_err(|_| PyRuntimeError::new_err("Failed to serialize value."))
}

/// Converts `obj` to the wire form of `object_type`. An error inside a
/// struct, an enum with data or a collection names the part that caused it.
pub(crate) fn serialize_py(
    obj: &Bound<PyAny>,
    object_type: &PyObjectType,
    creator: &mut ValueCreator,
) -> PyResult<()> {
    serialize(obj, object_type, creator).map_err(|failure| failure.into_error(obj.py()))
}

/// As [`serialize_py`], for one key, value or item of a collection that is
/// converted piece by piece; an error names the piece too.
pub(crate) fn serialize_py_in(
    obj: &Bound<PyAny>,
    object_type: &PyObjectType,
    creator: &mut ValueCreator,
    place: Place,
) -> PyResult<()> {
    serialize(obj, object_type, creator).map_err(|failure| {
        let step = match place {
            Place::Index(index) => Step::Index(index),
            Place::Value(key) => Step::Value(short_repr(key)),
            Place::Key => Step::Key(short_repr(obj)),
        };
        failure.inside([step]).into_error(obj.py())
    })
}

fn serialize(
    obj: &Bound<PyAny>,
    object_type: &PyObjectType,
    creator: &mut ValueCreator,
) -> Result<(), Failure> {
    match object_type {
        PyObjectType::U8 => add(creator, &obj.extract::<u8>()?)?,
        PyObjectType::U16 => add(creator, &obj.extract::<u16>()?)?,
        PyObjectType::U32 => add(creator, &obj.extract::<u32>()?)?,
        PyObjectType::U64 => add(creator, &obj.extract::<u64>()?)?,
        PyObjectType::I8 => add(creator, &obj.extract::<i8>()?)?,
        PyObjectType::I16 => add(creator, &obj.extract::<i16>()?)?,
        PyObjectType::I32 => add(creator, &obj.extract::<i32>()?)?,
        PyObjectType::I64 => add(creator, &obj.extract::<i64>()?)?,
        PyObjectType::F32 => add(creator, &obj.extract::<f32>()?)?,
        PyObjectType::F64 => add(creator, &obj.extract::<f64>()?)?,
        PyObjectType::Bool => add(creator, &obj.extract::<bool>()?)?,
        PyObjectType::String => add(creator, &obj.extract::<String>()?)?,
        PyObjectType::Enum(py_enum) => {
            let py_enum = py_enum.bind(obj.py());
            if !obj.is_instance(py_enum)? {
                return Err(PyTypeError::new_err(format!(
                    "Expected a member of {}, got {}",
                    class_name(py_enum, "__qualname__"),
                    short_repr(obj)
                ))
                .into());
            }
            let member_names = py_enum.getattr("_member_names_")?;
            let name = obj.getattr("name")?;
            let value = member_names.cast::<PyList>()?.index(name)? as u32;

            add(creator, &value)?
        }
        PyObjectType::Tuple(vec) => {
            let sequence = obj.cast::<PySequence>()?;
            let len = sequence.len()?;
            if len != vec.len() {
                return Err(PyValueError::new_err(format!(
                    "Expected a tuple of {} items, got {len}",
                    vec.len()
                ))
                .into());
            }

            for (i, item_type) in vec.iter().enumerate() {
                let item = sequence.get_item(i)?;
                serialize(&item, item_type, creator)
                    .map_err(|failure| failure.inside([Step::Index(i)]))?;
            }
        }
        PyObjectType::Class(fields, class) => {
            let class = class.bind(obj.py());
            if !obj.is_instance(class)? {
                return Err(PyTypeError::new_err(format!(
                    "Expected an instance of {}, got {}",
                    class_name(class, "__qualname__"),
                    short_repr(obj)
                ))
                .into());
            }
            for (name, field_type) in fields {
                let field = obj
                    .getattr(name.bind(obj.py()))
                    .map_err(Failure::from)
                    .and_then(|item| serialize(&item, field_type, creator));
                field.map_err(|failure| {
                    failure.inside([
                        Step::Attribute(name.bind(obj.py()).to_string()),
                        Step::Type(class_name(class, "__qualname__")),
                    ])
                })?;
            }
        }
        PyObjectType::List(size, items_type) => {
            let sequence = obj.cast::<PySequence>()?;
            let len = sequence.len()?;
            if len != *size as usize {
                return Err(PyValueError::new_err(format!(
                    "Expected a list of {size} items, got {len}"
                ))
                .into());
            }

            for (i, item) in sequence.try_iter()?.enumerate() {
                serialize(&item?, items_type, creator)
                    .map_err(|failure| failure.inside([Step::Index(i)]))?;
            }
        }
        PyObjectType::Vec(items_type) => {
            let list = obj.cast::<PyList>()?;
            add(creator, &(list.len() as u64))?;

            for (i, item) in list.iter().enumerate() {
                serialize(&item, items_type, creator)
                    .map_err(|failure| failure.inside([Step::Index(i)]))?;
            }
        }
        PyObjectType::Map(key_type, value_type) => {
            let dict = obj.cast::<PyDict>()?;
            add(creator, &(dict.len() as u64))?;

            for (key, value) in dict.iter() {
                serialize(&key, key_type, creator)
                    .map_err(|failure| failure.inside([Step::Key(short_repr(&key))]))?;
                serialize(&value, value_type, creator)
                    .map_err(|failure| failure.inside([Step::Value(short_repr(&key))]))?;
            }
        }
        PyObjectType::Option(object_type) => {
            if obj.is_none() {
                add(creator, &0u8)?;
            } else {
                add(creator, &1u8)?;
                serialize(obj, object_type, creator)?;
            }
        }
        PyObjectType::Empty => {}
        PyObjectType::DataEnum(class, variants) => {
            let class = class.bind(obj.py());
            let Some((index, variant)) = variants.iter().enumerate().find(|(_, v)| v.matches(obj))
            else {
                return Err(not_a_variant(class, variants, obj)?.into());
            };
            add(creator, &(index as u32))?;
            for (name, field_type) in &variant.fields {
                let field = obj
                    .getattr(name.bind(obj.py()))
                    .map_err(Failure::from)
                    .and_then(|item| serialize(&item, field_type, creator));
                field.map_err(|failure| {
                    failure.inside([
                        Step::Attribute(name.bind(obj.py()).to_string()),
                        Step::Attribute(class_name(variant.class.bind(obj.py()), "__name__")),
                        Step::Type(class_name(class, "__qualname__")),
                    ])
                })?;
            }
        }
    }
    Ok(())
}

pub(crate) fn deserialize_py<'py, 'a>(
    py: Python<'py>,
    parser: &'a mut ValueParser,
    object_type: &'a PyObjectType,
) -> PyResult<Bound<'py, PyAny>> {
    match object_type {
        PyObjectType::U8 => {
            let mut value = 0u8;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u8"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::U16 => {
            let mut value = 0u16;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u16"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::U32 => {
            let mut value = 0u32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u32"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::U64 => {
            let mut value = 0u64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u64"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::I8 => {
            let mut value = 0i8;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i8"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::I16 => {
            let mut value = 0i16;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i16"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::I32 => {
            let mut value = 0i32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i32"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::I64 => {
            let mut value = 0i64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i64"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::F32 => {
            let mut value = 0f32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse f32"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::F64 => {
            let mut value = 0f64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse f64"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::Bool => {
            let mut value = false;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse bool"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::String => {
            let mut value = String::new();
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse string"))?;
            value.into_bound_py_any(py)
        }
        PyObjectType::Enum(py_enum) => {
            let mut value = 0u32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse enum"))?;

            let obj = py_enum.bind(py);
            let name = obj
                .getattr("_member_names_")?
                .cast::<PyList>()?
                .get_item(value as usize)?;
            let enum_value = obj
                .getattr("_member_map_")?
                .cast::<PyDict>()?
                .get_item(name)?;

            let res = enum_value.ok_or(PyValueError::new_err("Failed to get enum member"))?;
            Ok(res.into_any())
        }
        PyObjectType::Tuple(vec) => {
            let mut items = Vec::with_capacity(vec.len());
            for item_type in vec.iter() {
                let item = deserialize_py(py, parser, item_type)?;
                items.push(item);
            }
            Ok(PyTuple::new(py, items)?.into_any())
        }
        PyObjectType::Class(fields, py_class) => {
            let mut items = Vec::with_capacity(fields.len());
            for (_, item_type) in fields {
                let item = deserialize_py(py, parser, item_type)?;
                items.push(item);
            }
            let tuple = PyTuple::new(py, items)?;
            py_class.bind(py).call1(tuple)
        }
        PyObjectType::List(size, items_type) => {
            let list = PyList::empty(py);

            for _ in 0..*size {
                let item = deserialize_py(py, parser, items_type)?;
                list.append(item)?;
            }

            Ok(list.into_any())
        }
        PyObjectType::Vec(items_type) => {
            let mut vec_size = 0u64;
            parser
                .get(&mut vec_size)
                .map_err(|_| PyValueError::new_err("Failed to parse vec size"))?;

            let list = PyList::empty(py);
            for _ in 0..vec_size {
                let item = deserialize_py(py, parser, items_type)?;
                list.append(item)?;
            }

            Ok(list.into_any())
        }
        PyObjectType::Map(key_type, value_type) => {
            let mut map_size = 0u64;
            parser
                .get(&mut map_size)
                .map_err(|_| PyValueError::new_err("Failed to parse map size"))?;

            let dict = PyDict::new(py);
            for _ in 0..map_size {
                let key = deserialize_py(py, parser, key_type)?;
                let value = deserialize_py(py, parser, value_type)?;
                dict.set_item(key, value)?;
            }

            Ok(dict.into_any())
        }
        PyObjectType::Option(object_type) => {
            let mut has_value = 0u8;
            parser
                .get(&mut has_value)
                .map_err(|_| PyValueError::new_err("Failed to parse option flag"))?;

            if has_value == 0 {
                Ok(PyNone::get(py).to_owned().into_any())
            } else {
                deserialize_py(py, parser, object_type)
            }
        }
        PyObjectType::Empty => Ok(PyTuple::empty(py).into_any()),
        PyObjectType::DataEnum(class, variants) => {
            let mut index = 0u32;
            parser
                .get(&mut index)
                .map_err(|_| PyValueError::new_err("Failed to parse enum variant"))?;
            let variant = variants.get(index as usize).ok_or_else(|| {
                PyValueError::new_err(format!("Unknown variant {index} of {}", class.bind(py)))
            })?;
            let mut items = Vec::with_capacity(variant.fields.len());
            for (_, item_type) in &variant.fields {
                items.push(deserialize_py(py, parser, item_type)?);
            }
            variant.class.bind(py).call1(PyTuple::new(py, items)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use pyo3::types::PyModule;

    use super::*;
    use crate::python::pytypes::{PyObjectClass, U32, cl};

    const STRUCTS: &std::ffi::CStr = cr#"
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int

@dataclass
class Other:
    x: int
    y: int

@dataclass
class Empty:
    pass

def reordered():
    point = Point(1, 2)
    del point.x
    point.x = 1
    return point
"#;

    fn class_type(
        module: &Bound<PyModule>,
        name: &str,
        fields: Vec<PyObjectClass>,
    ) -> PyObjectType {
        let py = module.py();
        let fields = fields
            .into_iter()
            .map(|field| Bound::new(py, field).unwrap())
            .collect();
        let class = module.getattr(name).unwrap().unbind();
        cl(py, fields, class).unwrap().object_type
    }

    fn roundtrip<'py>(obj: &Bound<'py, PyAny>, object_type: &PyObjectType) -> Bound<'py, PyAny> {
        let mut creator = ValueCreator::new();
        serialize_py(obj, object_type, &mut creator).unwrap();
        let mut parser = ValueParser::new(creator.finalize());
        deserialize_py(obj.py(), &mut parser, object_type).unwrap()
    }

    /// Fields are read by name, so an instance whose dict order differs from
    /// the field order still sends every value in its own field.
    #[test]
    fn class_fields_do_not_depend_on_the_instance_dict_order() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::from_code(py, STRUCTS, c"structs.py", c"structs").unwrap();
            let point = class_type(&module, "Point", vec![U32, U32]);

            let reordered = module.call_method0("reordered").unwrap();
            let state = reordered.getattr("__dict__").unwrap();
            let keys: Vec<String> = state.cast::<PyDict>().unwrap().keys().extract().unwrap();
            assert_eq!(keys, ["y", "x"]);

            let back = roundtrip(&reordered, &point);
            assert_eq!(back.getattr("x").unwrap().extract::<u32>().unwrap(), 1);
            assert_eq!(back.getattr("y").unwrap().extract::<u32>().unwrap(), 2);
            assert!(back.eq(&reordered).unwrap());

            let empty = class_type(&module, "Empty", vec![]);
            let instance = module.getattr("Empty").unwrap().call0().unwrap();
            assert!(roundtrip(&instance, &empty).eq(&instance).unwrap());
        });
    }

    /// An error names the field that caused it, through nested values, and
    /// keeps its exception type.
    #[test]
    fn an_error_names_the_field_that_caused_it() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::from_code(py, STRUCTS, c"structs.py", c"structs").unwrap();
            let point = class_type(&module, "Point", vec![U32, U32]);
            let wrong = module.getattr("Point").unwrap().call1((1, "two")).unwrap();

            let error = serialize_py(&wrong, &point, &mut ValueCreator::new()).unwrap_err();
            assert!(error.is_instance_of::<PyTypeError>(py), "{error}");
            let message = error.value(py).to_string();
            assert!(message.starts_with("Point.y: "), "{message}");

            let points = PyObjectType::Vec(Box::new(point.clone_py(py)));
            let list = PyList::new(
                py,
                [
                    module.getattr("Point").unwrap().call1((1, 2)).unwrap(),
                    wrong.clone(),
                ],
            )
            .unwrap();
            let error = serialize_py(&list, &points, &mut ValueCreator::new()).unwrap_err();
            let message = error.value(py).to_string();
            assert!(message.starts_with("[1].y: "), "{message}");

            let error = serialize_py_in(&wrong, &point, &mut ValueCreator::new(), Place::Index(3))
                .unwrap_err();
            let message = error.value(py).to_string();
            assert!(message.starts_with("[3].y: "), "{message}");

            // A value that is wrong as a whole has no path.
            let error = serialize_py(&list, &point, &mut ValueCreator::new()).unwrap_err();
            let message = error.value(py).to_string();
            assert!(
                message.starts_with("Expected an instance of Point, got ["),
                "{message}"
            );
        });
    }

    /// A struct value must be an instance of its class, even when another
    /// object has fields of the same names and types.
    #[test]
    fn class_rejects_instances_of_other_classes() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::from_code(py, STRUCTS, c"structs.py", c"structs").unwrap();
            let point = class_type(&module, "Point", vec![U32, U32]);

            let other = module.getattr("Other").unwrap().call1((1, 2)).unwrap();
            let tuple = PyTuple::new(py, [1, 2]).unwrap().into_any();
            for value in [other, tuple] {
                let error = serialize_py(&value, &point, &mut ValueCreator::new()).unwrap_err();
                assert!(error.is_instance_of::<PyTypeError>(py), "{error}");
                assert!(error.to_string().contains("Point"), "{error}");
            }

            let error = cl(
                py,
                vec![Bound::new(py, U32).unwrap()],
                module.getattr("Point").unwrap().unbind(),
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("__match_args__"), "{error}");
        });
    }
}
