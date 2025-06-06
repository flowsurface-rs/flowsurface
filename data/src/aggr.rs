pub mod ticks;
pub mod time;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickCount(pub u16);

impl TickCount {
    pub const ALL: [TickCount; 7] = [
        TickCount(10),
        TickCount(20),
        TickCount(50),
        TickCount(100),
        TickCount(200),
        TickCount(500),
        TickCount(1000),
    ];

    pub fn is_custom(&self) -> bool {
        !Self::ALL.contains(self)
    }
}

impl std::fmt::Display for TickCount {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}T", self.0)
    }
}
