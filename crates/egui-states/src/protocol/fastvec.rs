//! A byte buffer that stays on the stack while it is small, and the
//! postcard encoding into it.

use postcard::ser_flavors::Flavor;
use serde::Serialize;

pub(crate) struct StackVec<const N: usize>(pub(super) [u8; N], pub(super) usize);

impl<const N: usize> AsRef<[u8]> for StackVec<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..self.1]
    }
}

// FastVec ---------------------------------------------------------------
// -----------------------------------------------------------------------
pub(crate) enum FastVec<const N: usize> {
    Heap(Vec<u8>),
    Stack(StackVec<N>),
}

impl<const N: usize> FastVec<N> {
    #[inline]
    pub fn new() -> Self {
        Self::Stack(StackVec([0; N], 0))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn into_bytes(self) -> bytes::Bytes {
        match self {
            Self::Heap(vec) => bytes::Bytes::from_owner(vec),
            Self::Stack(vec) => bytes::Bytes::from_owner(vec),
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Heap(vec) => vec,
            Self::Stack(stack_vec) => stack_vec.0[..stack_vec.1].to_vec(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Heap(vec) => vec.len(),
            Self::Stack(stack_vec) => stack_vec.1,
        }
    }

    pub fn extend_from_data<const M: usize>(&mut self, data: &FastVec<M>) {
        match self {
            Self::Heap(vec) => match data {
                FastVec::Heap(dvec) => vec.extend_from_slice(dvec),
                FastVec::Stack(dvec) => vec.extend_from_slice(dvec.as_ref()),
            },
            Self::Stack(stack_vec) => match data {
                FastVec::Heap(dvec) => {
                    if stack_vec.1 + dvec.len() <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.len()].copy_from_slice(dvec);
                        stack_vec.1 += dvec.len();
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.len());
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(dvec);
                        *self = Self::Heap(new_vec);
                    }
                }
                FastVec::Stack(dvec) => {
                    if stack_vec.1 + dvec.1 <= N {
                        stack_vec.0[stack_vec.1..stack_vec.1 + dvec.1]
                            .copy_from_slice(&dvec.0[..dvec.1]);
                        stack_vec.1 += dvec.1;
                    } else {
                        let mut new_vec = Vec::with_capacity(stack_vec.1 + dvec.1);
                        new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                        new_vec.extend_from_slice(&dvec.0[..dvec.1]);
                        *self = Self::Heap(new_vec);
                    }
                }
            },
        }
    }
}

impl<const N: usize> Flavor for FastVec<N> {
    type Output = Self;

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.push(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 < N {
                    stack_vec.0[stack_vec.1] = data;
                    stack_vec.1 += 1;
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + 1);
                    new_vec.extend_from_slice(&stack_vec.0);
                    new_vec.push(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        match self {
            Self::Heap(vec) => {
                vec.extend_from_slice(data);
                Ok(())
            }
            Self::Stack(stack_vec) => {
                if stack_vec.1 + data.len() <= N {
                    stack_vec.0[stack_vec.1..stack_vec.1 + data.len()].copy_from_slice(data);
                    stack_vec.1 += data.len();
                    Ok(())
                } else {
                    let mut new_vec = Vec::with_capacity(stack_vec.1 + data.len());
                    new_vec.extend_from_slice(&stack_vec.0[..stack_vec.1]);
                    new_vec.extend_from_slice(data);
                    *self = Self::Heap(new_vec);
                    Ok(())
                }
            }
        }
    }

    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(self)
    }
}

#[inline]
pub(crate) fn serialize<T, const N: usize>(value: &T) -> Result<FastVec<N>, ()>
where
    T: Serialize,
{
    postcard::serialize_with_flavor::<T, FastVec<N>, FastVec<N>>(value, FastVec::new())
        .map_err(|_| ())
}

struct FastVecRef<'a, const N: usize>(&'a mut FastVec<N>);

impl<'a, const N: usize> Flavor for FastVecRef<'a, N> {
    type Output = ();

    #[inline]
    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0.try_push(data)
    }

    #[inline]
    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        self.0.try_extend(data)
    }

    #[inline]
    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(())
    }
}

#[inline]
pub(crate) fn serialize_to_data<T, const N: usize>(
    value: &T,
    data: &mut FastVec<N>,
) -> Result<(), ()>
where
    T: Serialize,
{
    let data_ref = FastVecRef(data);
    postcard::serialize_with_flavor::<T, FastVecRef<N>, ()>(value, data_ref).map_err(|_| ())
}

/// The bytes of `data`.
#[cfg(test)]
pub(super) fn contents<const N: usize>(data: &FastVec<N>) -> Vec<u8> {
    match data {
        FastVec::Heap(data) => data.clone(),
        FastVec::Stack(data) => data.as_ref().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic xorshift generator, so the model test needs no dependency.
    struct Random(u64);

    impl Random {
        fn next(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }

        fn bytes(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| self.next(256) as u8).collect()
        }
    }

    fn other<const M: usize>(bytes: &[u8], heap: bool) -> FastVec<M> {
        if heap {
            FastVec::Heap(bytes.to_vec())
        } else {
            let mut data = FastVec::<M>::new();
            data.try_extend(bytes).unwrap();
            data
        }
    }

    /// Every way of appending must behave like `Vec::extend`, before, at, and
    /// after the stack capacity is exhausted.
    #[test]
    fn fast_vec_appends_match_a_vec_model_across_the_stack_boundary() {
        const N: usize = 8;
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for _ in 0..500 {
            let mut data = FastVec::<N>::new();
            let mut model = Vec::new();
            for _ in 0..6 {
                match random.next(5) {
                    0 => {
                        let byte = random.next(256) as u8;
                        data.try_push(byte).unwrap();
                        model.push(byte);
                    }
                    1 => {
                        let len = random.next(2 * N);
                        let bytes = random.bytes(len);
                        data.try_extend(&bytes).unwrap();
                        model.extend(&bytes);
                    }
                    2 | 3 => {
                        // Stack and heap sources of a different capacity.
                        let len = random.next(N / 2 + 1);
                        let bytes = random.bytes(len);
                        let heap = random.next(2) == 0 || bytes.len() > N / 2;
                        data.extend_from_data(&other::<{ N / 2 }>(&bytes, heap));
                        model.extend(&bytes);
                    }
                    _ => {
                        let len = random.next(2 * N);
                        let bytes = random.bytes(len);
                        #[cfg(any(feature = "server", feature = "python"))]
                        data.extend_from_slice(&bytes);
                        #[cfg(not(any(feature = "server", feature = "python")))]
                        data.try_extend(&bytes).unwrap();
                        model.extend(&bytes);
                    }
                }
                assert_eq!(contents(&data), model);
                assert_eq!(data.len(), model.len());
            }
            #[cfg(not(target_arch = "wasm32"))]
            assert_eq!(data.into_bytes().as_ref(), model.as_slice());
        }
    }

    #[test]
    fn fast_vec_switches_to_the_heap_exactly_when_the_stack_is_full() {
        let mut data = FastVec::<4>::new();
        data.try_extend(&[1, 2, 3]).unwrap();
        data.try_push(4).unwrap();
        assert!(
            matches!(data, FastVec::Stack(_)),
            "four bytes fit the stack"
        );
        data.try_push(5).unwrap();
        assert!(matches!(data, FastVec::Heap(_)));
        assert_eq!(contents(&data), [1, 2, 3, 4, 5]);

        let mut data = FastVec::<4>::new();
        data.extend_from_data(&FastVec::<8>::Heap(vec![1, 2, 3, 4]));
        assert!(matches!(data, FastVec::Stack(_)));
        data.extend_from_data(&FastVec::<8>::Heap(vec![5]));
        assert!(matches!(data, FastVec::Heap(_)));
        assert_eq!(contents(&data), [1, 2, 3, 4, 5]);
    }
}
