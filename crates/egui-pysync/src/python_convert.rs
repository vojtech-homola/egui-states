use pyo3::{
    conversion::IntoPyObjectExt,
    prelude::*,
    types::{PyList, PyNone, PyString, PyTuple},
};

use crate::values::Empty;

pub trait ToPython: Send + Sync {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
}

#[cfg(feature = "server")]
#[derive(FromPyObject)]
pub enum EnumInit {
    Value(i64),
    Name(String),
}

macro_rules! impl_topython_basic {
    ($($t:ty),*) => {
        $(
            impl ToPython for $t {
                fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                    self.into_bound_py_any(py).unwrap()
                }
            }
        )*
    };
}

impl_topython_basic!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool);

// // None ---------------------------------------------------
// impl ToPython for () {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         PyNone::get(py).to_owned().into_any()
//     }
// }

// strings ---------------------------------------------------
impl ToPython for String {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        let ob = PyString::new(py, self);
        ob.into_any()
    }
}

// arrays ---------------------------------------------------
impl<T, const N: usize> ToPython for [T; N]
where
    T: ToPython,
{
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        let list = PyList::new(py, self.iter().map(|v| v.to_python(py))).unwrap();
        list.into_any()
    }
}

// tuples ---------------------------------------------------
macro_rules! impl_to_python_tuple {
    ($($idx:tt: $T:ident),*) => {
        impl<$($T),*> ToPython for ($($T,)*)
        where
            $($T: ToPython,)*
        {
            fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                let tuple = PyTuple::new(py, [$(&self.$idx.to_python(py)),*]).unwrap();
                tuple.into_any()
            }
        }
    };
}

impl_to_python_tuple!(0: T0, 1: T1);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

// EmptyValue ---------------------------------------------------
impl ToPython for Empty {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        PyNone::get(py).to_owned().into_any()
    }
}

impl<'py> FromPyObject<'py> for Empty {
    fn extract_bound(_: &Bound<'py, PyAny>) -> PyResult<Self> {
        Ok(Empty)
    }
}
