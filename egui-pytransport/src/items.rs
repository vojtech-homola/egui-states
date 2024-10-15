pub trait SyncItemWrite: Send + Sync + 'static {
    fn write(&self, data: &mut [u8]) -> Option<Vec<u8>>;
    fn size(&self) -> usize;
}

pub trait SyncItem: SyncItemWrite + Clone + 'static {
    fn read(data: &[u8]) -> Self;

    #[inline]
    fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
        Ok(match data {
            Some(data) => Self::read(&data),
            None => Self::read(head),
        })
    }
}

impl SyncItemWrite for bool {
    fn write(&self, data: &mut [u8]) -> Option<Vec<u8>> {
        data[0] = *self as u8;
        None
    }

    fn size(&self) -> usize {
        1
    }
}

impl SyncItem for bool {
    #[inline]
    fn read(data: &[u8]) -> Self {
        data[0] != 0
    }
}
