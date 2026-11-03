//! Torture test for saving and restoring (see vault_core::stress).
fn main() {
    std::process::exit(vault_core::stress::main_with_args());
}
