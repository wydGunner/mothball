//! Mothball's torture test for saving and restoring files (see vault_core::stress).
//!
//!   cargo run --release --manifest-path crates/vault-stress/Cargo.toml -- --rounds 2000
//!   ... -- --dir D:\MothballStress --rounds 2000      (test on a particular drive)
//!   ... -- --crash 25                                (kill it mid-work 25 times)
use libsqlite3_sys as _;

fn main() {
    std::process::exit(vault_core::stress::main_with_args());
}
