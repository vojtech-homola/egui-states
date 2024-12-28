use pyo3::{
    prelude::*,
    types::{PyNone, PyString, PyTuple},
};

pub trait ToPython: Send + Sync {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
}

macro_rules! impl_to_python_basic {
    ($($t:ty),*) => {
        $(
            impl ToPython for $t {
                fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                    self.into_pyobject(py).unwrap().into_any()
                }
            }
        )*
    };
}

impl_to_python_basic!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

impl ToPython for bool {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.into_pyobject(py).unwrap().to_owned().into_any()
    }
}

impl ToPython for () {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        PyNone::get(py).to_owned().into_any()
    }
}

impl ToPython for String {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        let ob = PyString::new(py, self);
        ob.into_any()
    }
}

// arrays ---------------------------------------------------
macro_rules! impl_to_python_array {
    ($t:ty, $($n:expr),*) => {
        $(
            impl ToPython for [$t; $n] {
                fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                    PyTuple::new(py, self).unwrap().into_any()
                }
            }
        )*
    };
}

impl_to_python_array!(bool, 2, 3, 4);
impl_to_python_array!(f32, 2, 3, 4);
impl_to_python_array!(f64, 2, 3, 4);
impl_to_python_array!(i32, 2, 3, 4);
impl_to_python_array!(i64, 2, 3, 4);
impl_to_python_array!(u32, 2, 3, 4);
impl_to_python_array!(u64, 2, 3, 4);
