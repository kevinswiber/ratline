pub mod bar;
pub mod box_model;
pub mod child;
pub mod dashboard_file;
pub mod dashboard_kdl;
pub mod datetime;
pub mod duration;
pub mod fuzzy;
pub mod join;
pub mod layout;
pub mod measure;
pub mod pager;
pub mod registry;
// The accumulator lands ahead of the reader that feeds it, so for now
// only its own tests construct one. The allow goes away with the first
// caller.
#[allow(dead_code)]
pub mod retain;
pub mod schedule;
pub mod snapshot;
pub mod spark;
pub mod table;
pub mod trigger;
