//! Back-compat facade. Real implementations live in `crate::pipelines`.
pub use crate::pipelines::research::{apply_accept, apply_reject, apply_write_back, render_result};
pub use crate::pipelines::{apply_claim, RUN_KEY, STATUS_KEY};
