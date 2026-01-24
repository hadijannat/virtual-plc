#![doc = "Real-time execution engine for the virtual PLC."]

pub mod io_image;
pub mod realtime;
pub mod scheduler;
pub mod wasm_host;
pub mod wasm_imports;
pub mod wasm_memory;
pub mod watchdog;

pub use io_image::*;
pub use realtime::*;
pub use scheduler::*;
pub use wasm_host::*;
pub use wasm_imports::HostState;
pub use wasm_memory::*;
pub use watchdog::*;
