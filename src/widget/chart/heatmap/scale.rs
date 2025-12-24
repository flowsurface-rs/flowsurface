pub mod axisx;
pub mod axisy;

pub use super::Message;

#[derive(Debug, Default)]
pub struct AxisInteraction {
    kind: AxisInteractionKind,
}

#[derive(Debug, Default)]
pub enum AxisInteractionKind {
    #[default]
    None,
    Panning {
        last_position: iced::Point,
    },
}

fn nice_step_i64(rough: i64) -> i64 {
    // Choose from 1,2,5 * 10^k
    let rough = rough.max(1);
    let mut pow10 = 1i64;
    while pow10.saturating_mul(10) <= rough {
        pow10 *= 10;
    }
    let m = (rough + pow10 - 1) / pow10; // ceil
    let mult = if m <= 1 {
        1
    } else if m <= 2 {
        2
    } else if m <= 5 {
        5
    } else {
        10
    };
    mult * pow10
}
