// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! A parser for the device tree blob format.
//!
//! The device tree blob format is a binary format used by firmware to describe non-discoverable
//! hardware to the operating system. This includes things like the number of CPUs and their frequency,
//! MMIO regions, interrupt controllers, and other platform-specific information.
//!
//! The format is described in detail in the [Device Tree Specification](https://github.com/devicetree-org/devicetree-specification);

#![no_std]

pub mod debug;
mod error;
mod parser;

use crate::parser::Parser;
use core::ffi::CStr;
use core::{mem, slice, str};
pub use error::Error;
use fallible_iterator::FallibleIterator;

type Result<T> = core::result::Result<T, Error>;

const FDT_BEGIN_NODE: u32 = 0x0000_0001;
const FDT_END_NODE: u32 = 0x0000_0002;
const FDT_PROP: u32 = 0x0000_0003;
const FDT_NOP: u32 = 0x0000_0004;
const FDT_END: u32 = 0x0000_0009;
const DTB_MAGIC: u32 = 0xD00D_FEED;
const DTB_VERSION: u32 = 17;

#[expect(
    unused_variables,
    clippy::missing_errors_doc,
    reason = "trait declaration"
)]
pub trait Visitor<'dt> {
    type Error: core::error::Error;

    fn visit_subnode(
        &mut self,
        name: &'dt str,
        node: Node<'dt>,
    ) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn visit_reg(&mut self, reg: &'dt [u8]) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn visit_address_cells(&mut self, cells: u32) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn visit_size_cells(&mut self, cells: u32) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn visit_compatible(&mut self, strings: Strings<'dt>) -> core::result::Result<(), Self::Error> {
        Ok(())
    }

    fn visit_property(
        &mut self,
        name: &'dt str,
        value: &'dt [u8],
    ) -> core::result::Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct DevTree<'dt> {
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    total_slice: &'dt [u8],
    memory_slice: &'dt [u8],
    parser: Parser<'dt>,
}

#[derive(Debug)]
#[repr(C)]
struct Header {
    magic: [u8; 4],
    totalsize: [u8; 4],
    off_dt_struct: [u8; 4],
    off_dt_strings: [u8; 4],
    off_mem_rsvmap: [u8; 4],
    version: [u8; 4],
    last_comp_version: [u8; 4],
    boot_cpuid_phys: [u8; 4],
    size_dt_strings: [u8; 4],
    size_dt_struct: [u8; 4],
}

impl<'dt> DevTree<'dt> {
    /// Parse a device tree blob starting at the given base pointer.
    ///
    /// # Safety
    ///
    /// The caller has to ensure the given pointer is valid and actually points to the device tree blob
    /// as only minimal sanity checking is performed.
    ///
    /// # Errors
    ///
    /// Returns an error if the magic or version fields are invalid.
    pub unsafe fn from_raw(base: *const u8) -> Result<Self> {
        // Safety: can't verify this is a valid pointer, caller has to uphold this invariant
        let header = unsafe { &*(base.cast::<Header>()) };

        if u32::from_be_bytes(header.magic) != DTB_MAGIC {
            return Err(Error::InvalidMagic);
        }

        if u32::from_be_bytes(header.version) != DTB_VERSION {
            return Err(Error::InvalidVersion);
        }

        // Safety: TODO should enforce limits and verify header values
        let struct_slice = unsafe {
            let addr = base.add(u32::from_be_bytes(header.off_dt_struct) as usize);
            let len = u32::from_be_bytes(header.size_dt_struct) as usize;
            slice::from_raw_parts(addr, len)
        };

        // Safety: TODO should enforce limits and verify header values
        let strings_slice = unsafe {
            let addr = base.add(u32::from_be_bytes(header.off_dt_strings) as usize);
            let length = u32::from_be_bytes(header.size_dt_strings) as usize;
            slice::from_raw_parts(addr, length)
        };

        // Safety: TODO should enforce limits and verify header values
        let memory_slice = unsafe {
            let addr = base.add(u32::from_be_bytes(header.off_mem_rsvmap) as usize);
            let length =
                u32::from_be_bytes(header.totalsize) - u32::from_be_bytes(header.off_mem_rsvmap);
            slice::from_raw_parts(addr, length as usize)
        };

        // Safety: TODO should enforce limits and verify header values
        let total_slice = unsafe {
            let length = u32::from_be_bytes(header.totalsize);
            slice::from_raw_parts(base, length as usize)
        };

        Ok(Self {
            version: u32::from_be_bytes(header.version),
            last_comp_version: u32::from_be_bytes(header.last_comp_version),
            boot_cpuid_phys: u32::from_be_bytes(header.boot_cpuid_phys),
            total_slice,
            memory_slice,
            parser: Parser {
                struct_slice,
                strings_slice,
                level: 0,
                offset: 0,
            },
        })
    }

    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn last_comp_version(&self) -> u32 {
        self.last_comp_version
    }

    #[must_use]
    pub fn boot_cpuid_phys(&self) -> u32 {
        self.boot_cpuid_phys
    }

    #[must_use]
    pub fn as_slice(&self) -> &'dt [u8] {
        self.total_slice
    }

    /// Visit the device tree blob with the given visitor.
    ///
    /// # Errors
    ///
    /// Returns an error if the visitor produces an error or the device tree blob is malformed.
    pub fn visit<E: core::error::Error + From<Error>>(
        mut self,
        visitor: &mut dyn Visitor<'dt, Error = E>,
    ) -> core::result::Result<(), E> {
        self.parser.visit(visitor)
    }

    #[must_use]
    pub fn reserved_entries(&self) -> ReserveEntries<'dt> {
        ReserveEntries {
            buf: self.memory_slice,
            offset: 0,
            done: false,
        }
    }
}

#[derive(Clone)]
pub struct Node<'dt> {
    parser: Parser<'dt>,
}

impl<'dt> Node<'dt> {
    fn new(struct_slice: &'dt [u8], strings_slice: &'dt [u8], offset: usize, level: usize) -> Self {
        Self {
            parser: Parser {
                struct_slice,
                strings_slice,
                offset,
                level,
            },
        }
    }

    /// # Errors
    ///
    /// Returns an error when parsing fails or when the visitor returned an error.
    pub fn visit<E: core::error::Error + From<Error>>(
        mut self,
        visitor: &mut dyn Visitor<'dt, Error = E>,
    ) -> core::result::Result<(), E> {
        self.parser.visit(visitor)
    }
}

/// # Errors
///
/// Returns an error if at the given offset there is no valid null-terminated utf-8 string.
pub fn read_str(slice: &[u8], offset: u32) -> Result<&str> {
    let slice = &slice.get(offset as usize..).ok_or(Error::UnexpectedEOF)?;
    let str = CStr::from_bytes_until_nul(slice)?;
    Ok(str.to_str()?)
}

#[derive(Debug)]
pub struct ReserveEntry {
    pub address: u64,
    pub size: u64,
}

pub struct ReserveEntries<'dt> {
    buf: &'dt [u8],
    offset: usize,
    done: bool,
}

impl ReserveEntries<'_> {
    fn read_u64(&mut self) -> Result<u64> {
        let bytes = self
            .buf
            .get(self.offset..self.offset + mem::size_of::<u64>())
            .ok_or(Error::UnexpectedEOF)?;
        self.offset += mem::size_of::<u64>();

        Ok(u64::from_be_bytes(bytes.try_into()?))
    }
}

impl FallibleIterator for ReserveEntries<'_> {
    type Item = ReserveEntry;
    type Error = Error;

    fn next(&mut self) -> core::result::Result<Option<Self::Item>, Self::Error> {
        if self.done || self.offset == self.buf.len() {
            Ok(None)
        } else {
            let entry = {
                let address = self.read_u64()?;
                let size = self.read_u64()?;

                Ok(ReserveEntry { address, size })
            };

            // entries where both address and size is zero mark the end
            let is_empty = entry.as_ref().is_ok_and(|e| e.address == 0 || e.size == 0);

            self.done = entry.is_err() || is_empty;

            if is_empty {
                Ok(None)
            } else {
                entry.map(Some)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Strings<'dt> {
    bytes: &'dt [u8],
    offset: usize,
    err: bool,
}

impl<'dt> Strings<'dt> {
    #[must_use]
    pub fn new(bytes: &'dt [u8]) -> Self {
        Self {
            bytes,
            offset: 0,
            err: false,
        }
    }
}

impl<'dt> FallibleIterator for Strings<'dt> {
    type Item = &'dt str;
    type Error = Error;

    fn next(&mut self) -> core::result::Result<Option<Self::Item>, Self::Error> {
        if self.offset == self.bytes.len() || self.err {
            return Ok(None);
        }

        let str = read_str(self.bytes, u32::try_from(self.offset)?)?;
        self.offset += str.len() + 1;

        Ok(Some(str))
    }
}
