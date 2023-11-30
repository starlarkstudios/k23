use crate::paging::VirtualAddress;

#[derive(Debug, onlyerror::Error)]
pub enum Error {
    #[error("failed to parse Device Tree Blob")]
    DTB(#[from] dtb_parser::Error),
    #[error("missing board info property: {0}")]
    MissingBordInfo(&'static str),
    #[error("SBI call failed: {0}")]
    SBI(#[from] sbicall::Error),
    #[error("virtual address {0:?} is too large to be mapped")]
    VirtualAddressTooLarge(VirtualAddress),
    #[error("virtual address {0:?} is not mapped")]
    VirtualAddressNotMapped(VirtualAddress),
    #[error("out of memory")]
    OutOfMemory,
}
