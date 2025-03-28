// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Supervisor Exception Program Counter Register

use super::{read_csr_as_usize, write_csr_as_usize};

read_csr_as_usize!(0x141);
write_csr_as_usize!(0x141);
