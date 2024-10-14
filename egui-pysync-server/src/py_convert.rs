use pyo3::prelude::*;

pub trait FromPyValue: Sized {
    fn from_python(obj: &Bound<PyAny>) -> PyResult<Self>;
}

macro_rules! impl_simpl_conversion {
    ($($t:ty),*) => {
        $(
            impl FromPyValue for $t {
                #[inline]
                fn from_python(obj: &Bound<PyAny>) -> PyResult<Self> {
                    obj.extract()
                }
            }
        )*
    };
}

impl_simpl_conversion!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, bool);
impl_simpl_conversion!(String);
impl_simpl_conversion!([f32; 2], [f64; 2]);
