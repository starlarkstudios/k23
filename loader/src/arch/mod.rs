// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

cfg_if::cfg_if! {
    if #[cfg(target_arch = "riscv64")] {
        mod riscv64;
        pub use riscv64::*;
        pub use riscv::*;
    } else {
        compile_error!("Unsupported target architecture");
    }
}
