use pyo3::exceptions::PyValueError;
use pyo3::types::{PyDict, PyList, PyNone, PyTuple};
use pyo3::{IntoPyObjectExt, prelude::*};

use crate::python::pytypes::ObjectType;
use crate::value_parsing::{ValueCreator, ValueParser};

pub(crate) fn serialize_py(
    obj: &Bound<PyAny>,
    object_type: &ObjectType,
    creator: &mut ValueCreator,
) -> PyResult<()> {
    match object_type {
        ObjectType::U8 => {
            let value: u8 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::U16 => {
            let value: u16 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::U32 => {
            let value: u32 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::U64 => {
            let value: u64 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::I8 => {
            let value: i8 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::I16 => {
            let value: i16 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::I32 => {
            let value: i32 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::I64 => {
            let value: i64 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::F32 => {
            let value: f32 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::F64 => {
            let value: f64 = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::Bool => {
            let value: bool = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::String => {
            let value: String = obj.extract()?;
            creator.add(&value);
        }
        ObjectType::Enum(_) => {
            let value: u32 = obj.call_method0("index")?.extract()?;
            creator.add(&value);
        }
        ObjectType::Tuple(vec) => {
            let tuple = obj.cast::<PyTuple>()?;
            if tuple.len() != vec.len() {
                return Err(PyValueError::new_err(
                    "Tuple length does not match the expected length",
                ));
            }

            for (i, item_type) in vec.iter().enumerate() {
                let item = tuple.get_item(i)?;
                serialize_py(&item, item_type, creator)?;
            }
        }
        ObjectType::Class(vec, _) => {
            let list = obj.call_method0("_get_values")?;
            for (i, item_type) in vec.iter().enumerate() {
                let item = list.get_item(i)?;
                serialize_py(&item, item_type, creator)?;
            }
        }
        ObjectType::List(size, items_type) => {
            let list = obj.cast::<pyo3::types::PyList>()?;
            if list.len() != *size as usize {
                return Err(PyValueError::new_err(
                    "List length does not match the expected length",
                ));
            }

            for item in list.iter() {
                serialize_py(&item, items_type, creator)?;
            }
        }
        ObjectType::Vec(items_type) => {
            let list = obj.cast::<pyo3::types::PyList>()?;
            creator.add(&(list.len() as u64));

            for item in list.iter() {
                serialize_py(&item, items_type, creator)?;
            }
        }
        ObjectType::Map(key_type, value_type) => {
            let dict = obj.cast::<pyo3::types::PyDict>()?;
            creator.add(&(dict.len() as u64));

            for (key, value) in dict.iter() {
                serialize_py(&key, key_type, creator)?;
                serialize_py(&value, value_type, creator)?;
            }
        }
        ObjectType::Option(object_type) => {
            if obj.is_none() {
                creator.add(&0u8);
            } else {
                creator.add(&1u8);
                serialize_py(obj, object_type, creator)?;
            }
        }
        ObjectType::Empty => {}
    }

    Ok(())
}

pub(crate) fn deserialize_py<'py, 'a>(
    py: Python<'py>,
    parser: &'a mut ValueParser,
    object_type: &'a ObjectType,
) -> PyResult<Bound<'py, PyAny>> {
    match object_type {
        ObjectType::U8 => {
            let mut value = 0u8;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u8"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::U16 => {
            let mut value = 0u16;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u16"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::U32 => {
            let mut value = 0u32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u32"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::U64 => {
            let mut value = 0u64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse u64"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::I8 => {
            let mut value = 0i8;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i8"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::I16 => {
            let mut value = 0i16;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i16"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::I32 => {
            let mut value = 0i32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i32"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::I64 => {
            let mut value = 0i64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse i64"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::F32 => {
            let mut value = 0f32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse f32"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::F64 => {
            let mut value = 0f64;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse f64"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::Bool => {
            let mut value = false;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse bool"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::String => {
            let mut value = String::new();
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse string"))?;
            value.into_bound_py_any(py)
        }
        ObjectType::Enum(py_enum) => {
            let mut value = 0u32;
            parser
                .get(&mut value)
                .map_err(|_| PyValueError::new_err("Failed to parse enum"))?;

            py_enum.bind(py).call_method1("from_index", (value,))
        }
        ObjectType::Tuple(vec) => {
            let mut items = Vec::with_capacity(vec.len());
            for item_type in vec.iter() {
                let item = deserialize_py(py, parser, item_type)?;
                items.push(item);
            }
            Ok(PyTuple::new(py, items)?.into_any())
        }
        ObjectType::Class(vec, py_class) => {
            let mut items = Vec::with_capacity(vec.len());
            for item_type in vec.iter() {
                let item = deserialize_py(py, parser, item_type)?;
                items.push(item);
            }
            let tuple = PyTuple::new(py, items)?;
            py_class.bind(py).call1(tuple)
        }
        ObjectType::List(size, items_type) => {
            let list = PyList::empty(py);

            for _ in 0..*size {
                let item = deserialize_py(py, parser, items_type)?;
                list.append(item)?;
            }

            Ok(list.into_any())
        }
        ObjectType::Vec(items_type) => {
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
        ObjectType::Map(key_type, value_type) => {
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
        ObjectType::Option(object_type) => {
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
        ObjectType::Empty => Ok(PyTuple::empty(py).into_any()),
    }
}
