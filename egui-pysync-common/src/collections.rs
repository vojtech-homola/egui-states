pub trait ItemWriteRead: Send + Sync + Clone + 'static {
    fn write(&self, head: &mut [u8]);
    fn read(head: &[u8]) -> Self;
    fn size() -> usize;
}

impl ItemWriteRead for bool {
    #[inline]
    fn write(&self, head: &mut [u8]) {
        head[0] = *self as u8;
    }

    #[inline]
    fn read(head: &[u8]) -> Self {
        head[0] != 0
    }

    #[inline]
    fn size() -> usize {
        1
    }
}

macro_rules! impl_basic_item {
    ($($t:ty),*) => {
        $(
            impl ItemWriteRead for $t {
                #[inline]
                fn write(&self, head: &mut [u8]) {
                    head[0..std::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
                }

                #[inline]
                fn read(head: &[u8]) -> Self {
                    Self::from_le_bytes(head[0..std::mem::size_of::<$t>()].try_into().unwrap())
                }

                #[inline]
                fn size() -> usize {
                    std::mem::size_of::<$t>()
                }
            }
        )*
    };
}

impl_basic_item!(i64, u64, f64);
impl_basic_item!(i32, u32, f32);
impl_basic_item!(i16, u16);
impl_basic_item!(i8, u8);

macro_rules! impl_two_item {
    ($($t:ty),*) => {
        $(
            impl ItemWriteRead for [$t; 2] {
                fn write(&self, head: &mut [u8]) {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    const SIZE2: usize = 2 * SIZE;
                    head[0..SIZE].copy_from_slice(&self[0].to_le_bytes());
                    head[SIZE..SIZE2].copy_from_slice(&self[1].to_le_bytes());
                }

                fn read(head: &[u8]) -> Self {
                    const SIZE: usize = std::mem::size_of::<$t>();
                    const SIZE2: usize = 2 * SIZE;
                    [
                        <$t>::from_le_bytes(head[0..SIZE].try_into().unwrap()),
                        <$t>::from_le_bytes(head[SIZE..SIZE2].try_into().unwrap()),
                    ]
                }

                fn size() -> usize {
                    2 * std::mem::size_of::<$t>()
                }
            }
        )*
    };
}

impl_two_item!(i64, u64, f64, f32);

// macro_rules! impl_basic_item {
//     ($($t:ty),*) => {
//         $(
//             impl ItemWrite for $t {
//                 fn write(&self, head: &mut [u8]) {
//                     head[0..std::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
//                 }
//             }

//             impl ItemRead for $t {
//                 fn read(head: &[u8]) -> Self {
//                     Self::from_le_bytes(head[0..std::mem::size_of::<$t>()].try_into().unwrap())
//                 }
//             }
//         )*
//     };
// }

// impl_basic_item!(i64, u64, f64);
// impl_basic_item!(i32, u32, f32);
// impl_basic_item!(i16, u16);
// impl_basic_item!(i8, u8);

// macro_rules! impl_two_item {
//     ($($t:ty),*) => {
//         $(
//             impl ItemWrite for [$t; 2] {
//                 fn write(&self, head: &mut [u8]) {
//                     const SIZE: usize = std::mem::size_of::<$t>();
//                     const SIZE2: usize = 2 * SIZE;
//                     head[0..SIZE].copy_from_slice(&self[0].to_le_bytes());
//                     head[SIZE..SIZE2].copy_from_slice(&self[1].to_le_bytes());
//                 }
//             }

//             impl ItemRead for [$t; 2] {
//                 fn read(head: &[u8]) -> Self {
//                     const SIZE: usize = std::mem::size_of::<$t>();
//                     const SIZE2: usize = 2 * SIZE;
//                     [
//                         <$t>::from_le_bytes(head[0..SIZE].try_into().unwrap()),
//                         <$t>::from_le_bytes(head[SIZE..SIZE2].try_into().unwrap()),
//                     ]
//                 }
//             }
//         )*
//     };
// }

// impl_two_item!(i64, u64, f64);
