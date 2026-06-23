//! Core domain types and traits for The Construct.

pub mod clock;
pub mod model;
pub mod stage;
pub mod store;
pub mod tool;
pub mod types;

pub fn construct_name() -> &'static str {
    "The Construct"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_set() {
        assert_eq!(construct_name(), "The Construct");
    }
}
