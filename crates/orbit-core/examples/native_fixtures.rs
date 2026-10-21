//! Generates synthetic native fixtures through the production wire types.
#[path = "../tests/support/native_fixture_generator.rs"]
mod native_fixture_generator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    native_fixture_generator::generate()
}
