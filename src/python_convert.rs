use pyo3::{conversion::IntoPyObjectExt, prelude::*};

pub trait ToPython: Send + Sync {
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny>;
}

// macro_rules! impl_allow_basic {
//     ($($t:ty),*) => {
//         $(
//             impl AllowedValues for $t {}
//         )*
//     };
// }

// impl_allow_basic!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool, ());

// // bool ---------------------------------------------------
// impl ToPython for bool {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         self.into_pyobject(py).unwrap().to_owned().into_any()
//     }
// }

// None ---------------------------------------------------
// impl ToPython for () {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         PyNone::get(py).to_owned().into_any()
//     }
// }

// strings ---------------------------------------------------
// impl ToPython for String {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         let ob = PyString::new(py, self);
//         ob.into_any()
//     }
// }

// arrays ---------------------------------------------------
// impl<T, const N: usize> ToPython for [T; N]
// where
//     T: ToPython,
// {
//     fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
//         let list = PyList::new(py, self.iter().map(|v| v.to_python(py))).unwrap();
//         list.into_any()
//     }
// }

// tuples ---------------------------------------------------
// macro_rules! impl_allow_tuple {
//     ($($T:ident),*) => {
//         impl<$($T),*> AllowedValues for ($($T,)*)
//         where
//             $($T: ToPython,)*
//         {}
//     };
// }

// impl_allow_tuple!(T0, T1);
// impl_allow_tuple!(T0, T1, T2);
// impl_allow_tuple!(T0, T1, T2, T3);
// impl_allow_tuple!(T0, T1, T2, T3, T4);
// impl_allow_tuple!(T0, T1, T2, T3, T4, T5);
// impl_allow_tuple!(T0, T1, T2, T3, T4, T5, T6);
// impl_allow_tuple!(T0, T1, T2, T3, T4, T5, T6, T7);
// impl_allow_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
// impl_allow_tuple!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);

// impl_to_python_tuple!(0: T0, 1: T1);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8);
// impl_to_python_tuple!(0: T0, 1: T1, 2: T2, 3: T3, 4: T4, 5: T5, 6: T6, 7: T7, 8: T8, 9: T9);

impl<T> ToPython for T
where
    T: for<'py> IntoPyObjectExt<'py> + Send + Sync + Clone,
{
    fn to_python<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.clone().into_bound_py_any(py).unwrap()
    }
}
