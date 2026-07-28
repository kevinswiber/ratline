pub mod bar;
pub mod box_model;
pub mod datetime;
pub mod duration;
pub mod fuzzy;
pub mod join;
pub mod measure;
pub mod pager;
// Wired into the watch loop later in this change series; the allow
// keeps the standalone module honest until then.
#[allow(dead_code)]
pub mod schedule;
pub mod snapshot;
pub mod spark;
pub mod table;
