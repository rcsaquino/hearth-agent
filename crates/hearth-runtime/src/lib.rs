//! Hearth runtime wiring.

mod doctor;
mod init;

pub use doctor::{DoctorError, doctor};
pub use init::{InitError, InitOptions, init};
