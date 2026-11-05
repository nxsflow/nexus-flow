//! Every group of cases, for the two binaries that run them.

#[path = "endings.rs"]
pub mod endings;
#[path = "escalation.rs"]
pub mod escalation;
#[path = "fan_out.rs"]
pub mod fan_out;
#[path = "happy.rs"]
pub mod happy;
#[path = "liveness.rs"]
pub mod liveness;
#[path = "races.rs"]
pub mod races;
#[path = "rework.rs"]
pub mod rework;
#[path = "root.rs"]
pub mod root;
#[path = "status.rs"]
pub mod status;
#[path = "tree.rs"]
pub mod tree;
#[path = "wake.rs"]
pub mod wake;
#[path = "wind_down.rs"]
pub mod wind_down;
#[path = "withdraw.rs"]
pub mod withdraw;

type Case = (&'static str, fn(&mut crate::script::Script));

pub const ALL: &[&[Case]] = &[
    endings::ALL,
    escalation::ALL,
    fan_out::ALL,
    happy::ALL,
    liveness::ALL,
    races::ALL,
    rework::ALL,
    root::ALL,
    status::ALL,
    tree::ALL,
    wake::ALL,
    wind_down::ALL,
    withdraw::ALL,
];
