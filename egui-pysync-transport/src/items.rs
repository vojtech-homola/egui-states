pub trait ReadItem: Send + Sync + Clone + 'static {
    fn read(head: &[u8]) -> Self;
}

pub trait WriteItem: Send + Sync + Clone + 'static {
    fn write(&self, head: &mut [u8]);
}