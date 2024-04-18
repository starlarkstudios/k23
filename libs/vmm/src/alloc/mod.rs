mod bitmap;
mod bump;

use crate::PhysicalAddress;

pub use bitmap::BitMapAllocator;
pub use bump::BumpAllocator;

#[derive(Debug)]
pub struct FrameUsage {
    pub used: usize,
    pub total: usize,
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> crate::Result<PhysicalAddress> {
        self.allocate_frames(1)
    }
    fn allocate_frames(&mut self, frames: usize) -> crate::Result<PhysicalAddress>;
    fn deallocate_frame(&mut self, base: PhysicalAddress) -> crate::Result<()> {
        self.deallocate_frames(base, 1)
    }
    fn deallocate_frames(&mut self, base: PhysicalAddress, frames: usize) -> crate::Result<()>;

    /// Information about the number of physical frames used, and available
    fn frame_usage(&self) -> FrameUsage;
}
