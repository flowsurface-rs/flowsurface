pub mod ticks;
pub mod time;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickCount {
    T10,
    T20,
    T50,
    T100,
    T200,
    T500,
    T1000,
}

impl TickCount {
    pub const ALL: [TickCount; 7] = [
        TickCount::T10,
        TickCount::T20,
        TickCount::T50,
        TickCount::T100,
        TickCount::T200,
        TickCount::T500,
        TickCount::T1000,
    ];
}

impl From<usize> for TickCount {
    fn from(value: usize) -> Self {
        match value {
            10 => TickCount::T10,
            20 => TickCount::T20,
            50 => TickCount::T50,
            100 => TickCount::T100,
            200 => TickCount::T200,
            500 => TickCount::T500,
            1000 => TickCount::T1000,
            _ => panic!("Invalid tick count value"),
        }
    }
}

impl From<TickCount> for u64 {
    fn from(value: TickCount) -> Self {
        match value {
            TickCount::T10 => 10,
            TickCount::T20 => 20,
            TickCount::T50 => 50,
            TickCount::T100 => 100,
            TickCount::T200 => 200,
            TickCount::T500 => 500,
            TickCount::T1000 => 1000,
        }
    }
}

impl From<u64> for TickCount {
    fn from(value: u64) -> Self {
        match value {
            10 => TickCount::T10,
            20 => TickCount::T20,
            50 => TickCount::T50,
            100 => TickCount::T100,
            200 => TickCount::T200,
            500 => TickCount::T500,
            1000 => TickCount::T1000,
            _ => panic!("Invalid tick count value"),
        }
    }
}

impl std::fmt::Display for TickCount {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}T", u64::from(*self))
    }
}
