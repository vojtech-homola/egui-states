pub trait CollectionItem: Send + Sync + Clone + 'static {
    // for dynamic it is 0
    // max size is u16::MAX
    const SIZE: usize;

    fn read_item(data: &[u8]) -> Self;

    // implement this method if the type is static
    #[allow(unused_variables)]
    fn write_static(&self, data: &mut [u8]) {
        panic!("This type is not static");
    }

    // implement this method if the type is dynamic
    fn get_dynamic(&self) -> Vec<u8> {
        panic!("This type is not dynamic");
    }
}

impl CollectionItem for bool {
    const SIZE: usize = 1;

    #[inline]
    fn write_static(&self, data: &mut [u8]) {
        data[0] = *self as u8;
    }

    #[inline]
    fn read_item(data: &[u8]) -> Self {
        data[0] != 0
    }
}

macro_rules! impl_basic_item {
    ($($t:ty),*) => {
        $(
            impl CollectionItem for $t {
                const SIZE: usize = std::mem::size_of::<$t>();

                #[inline]
                fn write_static(&self, data: &mut [u8]) {
                    data[0..std::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
                }

                #[inline]
                fn read_item(data: &[u8]) -> Self {
                    Self::from_le_bytes(data[0..std::mem::size_of::<$t>()].try_into().unwrap())
                }
            }
        )*
    };
}

impl_basic_item!(i64, u64, f64);
impl_basic_item!(i32, u32, f32);
impl_basic_item!(i16, u16);
impl_basic_item!(i8, u8);

macro_rules! impl_2_array {
    ($($t:ty),*) => {
        $(
            impl CollectionItem for [$t; 2] {
                const SIZE: usize = 2 * std::mem::size_of::<$t>();

                fn write_static(&self, data: &mut [u8]) {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    const SIZE2: usize = 2 * SIZE;
                    data[0..SIZE].copy_from_slice(&self[0].to_le_bytes());
                    data[SIZE..SIZE2].copy_from_slice(&self[1].to_le_bytes());
                }

                fn read_item(data: &[u8]) -> Self {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    const SIZE2: usize = 2 * SIZE;
                    [
                        <$t>::from_le_bytes(data[0..SIZE].try_into().unwrap()),
                        <$t>::from_le_bytes(data[SIZE..SIZE2].try_into().unwrap()),
                    ]
                }
            }
        )*
    };
}

impl_2_array!(i64, u64, f64, f32);
