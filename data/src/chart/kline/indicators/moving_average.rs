use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MovingAverageMethod {
    #[default]
    Sma,
    Ema,
    Vwma,
}

impl MovingAverageMethod {
    pub fn calculate(self, values: &[f32], weights: &[f32]) -> Option<f32> {
        if values.is_empty() {
            return None;
        }

        match self {
            Self::Sma => Some(values.iter().sum::<f32>() / values.len() as f32),
            Self::Ema => {
                let alpha = 2.0 / (values.len() as f32 + 1.0);
                let mut average = values[0];
                for value in &values[1..] {
                    average += alpha * (*value - average);
                }
                Some(average)
            }
            Self::Vwma => {
                let mut weighted_sum = 0.0;
                let mut total_weight = 0.0;
                for (value, weight) in values.iter().zip(weights.iter()) {
                    weighted_sum += value * weight;
                    total_weight += weight;
                }

                if total_weight > 0.0 {
                    Some(weighted_sum / total_weight)
                } else {
                    Self::Sma.calculate(values, weights)
                }
            }
        }
    }
}
