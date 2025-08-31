use serde::{Deserialize, Serialize};

// graphs -------------------------------------------------------------
pub trait GraphElement: Clone + Copy + Send + Sync + 'static {
    fn zero() -> Self;
}

#[derive(Clone)]
pub struct Graph<T> {
    pub y: Vec<T>,
    pub x: Option<Vec<T>>,
}

impl<T: GraphElement> Graph<T> {
    pub fn to_graph_data(&self) -> (GraphDataInfo<T>, Vec<u8>) {
        let bytes_size = std::mem::size_of::<T>() * self.y.len();
        let points = self.y.len();

        match self.x {
            Some(ref x) => {
                let mut data = vec![0u8; bytes_size * 2];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = x.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[..bytes_size].copy_from_slice(dat_slice);

                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data[bytes_size..].copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                (GraphDataInfo::new(points, false), data)
            }

            None => {
                let mut data = vec![0u8; bytes_size];
                #[cfg(target_endian = "little")]
                {
                    let dat_slice = unsafe {
                        let ptr = self.y.as_ptr() as *const u8;
                        std::slice::from_raw_parts(ptr, bytes_size)
                    };
                    data.copy_from_slice(dat_slice);
                }

                // TODO: implement big endian
                #[cfg(target_endian = "big")]
                {
                    unimplemented!("Big endian not implemented yet.");
                }

                (GraphDataInfo::new(points, true), data)
            }
        }
    }

    pub fn add_points_from_data(
        &mut self,
        info: GraphDataInfo<T>,
        data: &[u8],
    ) -> Result<(), String> {
        let GraphDataInfo {
            points, is_linear, ..
        } = info;

        #[cfg(target_endian = "little")]
        {
            match (&mut self.x, is_linear) {
                (Some(x), false) => {
                    let old_size = x.len();
                    x.resize(old_size + points, T::zero());
                    let mut ptr = data.as_ptr() as *const T;
                    let data_slice = unsafe { std::slice::from_raw_parts(ptr, points) };
                    x[old_size..].copy_from_slice(data_slice);

                    self.y.resize(old_size + points, T::zero());
                    let data_slice = unsafe {
                        ptr = ptr.add(points);
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                (None, true) => {
                    let old_size = self.y.len();
                    self.y.resize(old_size + points, T::zero());
                    let data_slice = unsafe {
                        let ptr = data.as_ptr() as *const T;
                        std::slice::from_raw_parts(ptr, points)
                    };
                    self.y[old_size..].copy_from_slice(data_slice);

                    Ok(())
                }
                _ => return Err("Incoming Graph data and graph are not compatible.".to_string()),
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented yet.");
        }
    }

    pub fn from_graph_data(info: GraphDataInfo<T>, data: &[u8]) -> Self {
        let GraphDataInfo {
            is_linear, points, ..
        } = info;

        #[cfg(target_endian = "little")]
        {
            match is_linear {
                true => {
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let y_ptr = y.as_mut_ptr() as *mut u8;
                    let bytes = points * size_of::<T>();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data.as_ptr(), y_ptr, bytes);
                        y.set_len(points);
                    }

                    Graph { x: None, y }
                }
                false => {
                    let bytes = points * size_of::<T>();
                    let mut x: Vec<T> = Vec::with_capacity(points);
                    let ptr = x.as_mut_ptr() as *mut u8;
                    let mut data_ptr = data.as_ptr();
                    unsafe {
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        x.set_len(points);
                    }
                    let mut y: Vec<T> = Vec::with_capacity(points);
                    let ptr = y.as_mut_ptr() as *mut u8;
                    unsafe {
                        data_ptr = data_ptr.add(bytes);
                        std::ptr::copy_nonoverlapping(data_ptr, ptr, bytes);
                        y.set_len(points);
                    }

                    Graph { x: Some(x), y }
                }
            }
        }

        #[cfg(target_endian = "big")]
        {
            unimplemented!("Big endian not implemented yet.");
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct GraphDataInfo<T> {
    phantom: std::marker::PhantomData<T>,
    is_linear: bool,
    points: usize,
}

impl<T> GraphDataInfo<T> {
    pub fn new(points: usize, is_linear: bool) -> Self {
        Self {
            phantom: std::marker::PhantomData,
            is_linear,
            points,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum GraphMessage<T> {
    Set(u16, GraphDataInfo<T>),
    AddPoints(u16, GraphDataInfo<T>),
    Remove(u16),
    Reset,
}
