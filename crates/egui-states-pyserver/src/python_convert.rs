use pyo3::{
    conversion::IntoPyObjectExt,
    prelude::*,
    types::{PyList, PyNone, PyString, PyTuple},
};

// use egui_states_core::empty::Empty;

pub trait ToPython: Send + Sync {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
}

// only use for signals because FromPyObject is not implemented for ()
pub trait FromPython: Sized {
    fn from_python(obj: &Bound<PyAny>) -> PyResult<Self>;
}

#[derive(FromPyObject)]
pub enum EnumInit {
    Value(i64),
    Name(String),
}

impl FromPython for EnumInit {
    #[inline]
    fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
        obj.extract()
    }
}

macro_rules! impl_topython_basic {
    ($($t:ty),*) => {
        $(
            impl ToPython for $t {
                #[inline]
                fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                    self.into_bound_py_any(py).unwrap()
                }
            }
        )*
    };
}

impl_topython_basic!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool);

macro_rules! impl_frompython_basic {
    ($($t:ty),*) => {
        $(
            impl FromPython for $t {
                #[inline]
                fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
                    obj.extract()
                }
            }
        )*
    };
}

impl_frompython_basic!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool);

// Empty ------------------------------------------------------
impl ToPython for () {
    #[inline]
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        PyNone::get(py).to_owned().into_any()
    }
}

impl FromPython for () {
    #[inline]
    fn from_python(_: &Bound<PyAny>) -> PyResult<Self> {
        Ok(())
    }
}

// strings ---------------------------------------------------
impl ToPython for String {
    #[inline]
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        PyString::new(py, self).into_any()
    }
}

impl FromPython for String {
    #[inline]
    fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
        obj.extract()
    }
}

// arrays ---------------------------------------------------
impl<T, const N: usize> ToPython for [T; N]
where
    T: ToPython,
{
    #[inline]
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        PyList::new(py, self.iter().map(|v| v.to_python(py)))
            .unwrap()
            .into_any()
    }
}

impl<T, const N: usize> FromPython for [T; N]
where
    T: for<'a, 'py> FromPyObject<'a, 'py>,
{
    #[inline]
    fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
        obj.extract()
    }
}

// tuples ---------------------------------------------------
macro_rules! impl_to_python_tuple {
    ($($idx:tt: $T:ident),*) => {
        impl<$($T),*> ToPython for ($($T,)*)
        where
            $($T: ToPython,)*
        {
            #[inline]
            fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
                PyTuple::new(py, [$(&self.$idx.to_python(py)),*]).unwrap().into_any()
            }
        }
    };
}

impl_to_python_tuple!(0: T0);
impl_to_python_tuple!(0: T0, 1: T1);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

macro_rules! impl_from_python_tuple {
    ($($idx:tt: $T:ident),*) => {
        impl<$($T),*> FromPython for ($($T,)*)
        where
            $($T: for<'a, 'py> FromPyObject<'a, 'py>,)*
        {
            #[inline]
            fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
                obj.extract()
            }
        }
    };
}

impl_from_python_tuple!(0: T0);
impl_from_python_tuple!(0: T0, 1: T1);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
impl_from_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

// // EmptyValue ---------------------------------------------------
// impl ToPython for Empty {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         PyNone::get(py).to_owned().into_any()
//     }
// }

// impl<'py> FromPyObject<'py> for Empty {
//     fn extract_bound(_: &Bound<'py, PyAny>) -> PyResult<Self> {
//         Ok(Empty)
//     }
// }
