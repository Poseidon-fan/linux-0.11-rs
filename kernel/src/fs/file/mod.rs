/// Generic opened file object in kernel.
pub trait File: Send + Sync {
    fn read(&self, buffer: &mut [u8]) -> Result<usize, u32>;
    fn write(&self, buffer: &[u8]) -> Result<usize, u32>;
}
