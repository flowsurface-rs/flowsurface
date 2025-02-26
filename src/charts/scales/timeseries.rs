const M1_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 720, // 12 hour
    1000 * 60 * 180, // 3 hour
    1000 * 60 * 60,  // 1 hour
    1000 * 60 * 30,  // 30 min
    1000 * 60 * 15,  // 15 min
    1000 * 60 * 10,  // 10 min
    1000 * 60 * 5,   // 5 min
    1000 * 60 * 2,   // 2 min
    60 * 1000,       // 1 min
];

const M3_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 180,  // 6 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 9,    // 9 min
    1000 * 60 * 3,    // 3 min
];

const M5_TIME_STEPS: [u64; 9] = [
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
    1000 * 60 * 30,   // 30 min
    1000 * 60 * 15,   // 15 min
    1000 * 60 * 5,    // 5 min
];

const HOURLY_TIME_STEPS: [u64; 8] = [
    1000 * 60 * 5760, // 96 hour
    1000 * 60 * 2880, // 48 hour
    1000 * 60 * 1440, // 24 hour
    1000 * 60 * 720,  // 12 hour
    1000 * 60 * 480,  // 8 hour
    1000 * 60 * 240,  // 4 hour
    1000 * 60 * 120,  // 2 hour
    1000 * 60 * 60,   // 1 hour
];

const MS_TIME_STEPS: [u64; 8] = [
    1000 * 30,
    1000 * 10,
    1000 * 5,
    1000 * 2,
    1000,
    500,
    200,
    100,
];

pub fn calc_time_step(earliest: u64, latest: u64, labels_can_fit: i32, timeframe: u64) -> (u64, u64) {
    let timeframe_in_min = timeframe / 60000;

    let time_steps: &[u64] = match timeframe_in_min {
        0_u64..1_u64 => &MS_TIME_STEPS,
        1..=30 => match timeframe_in_min {
            1 => &M1_TIME_STEPS,
            3 => &M3_TIME_STEPS,
            5 => &M5_TIME_STEPS,
            15 => &M5_TIME_STEPS[..7],
            30 => &M5_TIME_STEPS[..6],
            _ => &HOURLY_TIME_STEPS,
        },
        31.. => &HOURLY_TIME_STEPS,
    };

    let duration = latest - earliest;
    let mut selected_step = time_steps[0];

    for &step in time_steps {
        if duration / step >= (labels_can_fit as u64) {
            selected_step = step;
            break;
        }
        if step <= duration {
            selected_step = step;
        }
    }

    let rounded_earliest = (earliest / selected_step) * selected_step;

    (selected_step, rounded_earliest)
}