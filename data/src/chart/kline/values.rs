#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorDataFieldKey {
    SignedOverlay,
    Custom(&'static str),
}

pub const BOLLINGER_UPPER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.upper");
pub const BOLLINGER_LOWER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.lower");

pub const MOVING_AVERAGES_MEDIUM_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("moving_averages.medium");
pub const MOVING_AVERAGES_SLOW_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("moving_averages.slow");

pub const BAR_ANALYSIS_BUY_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bar_analysis.buy");
pub const BAR_ANALYSIS_SELL_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bar_analysis.sell");
pub const BAR_ANALYSIS_TOTAL_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bar_analysis.total");
pub const BAR_ANALYSIS_DELTA_PCT_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bar_analysis.delta_pct");

pub const VOLUME_BUY_FIELD_KEY: IndicatorDataFieldKey = IndicatorDataFieldKey::Custom("volume.buy");
pub const VOLUME_SELL_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("volume.sell");

pub const RSI_UPPER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.upper_band");
pub const RSI_SIGNAL_FIELD_KEY: IndicatorDataFieldKey = IndicatorDataFieldKey::Custom("rsi.signal");
pub const RSI_LOWER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.lower_band");

pub const STOCH_RSI_D_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("stoch_rsi.d");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorCellValue {
    Number(f32),
    Boolean(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorDataField {
    pub key: IndicatorDataFieldKey,
    pub value: IndicatorCellValue,
}

const MAX_INDICATOR_EXTRA_FIELDS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorData {
    value: IndicatorCellValue,
    extra_fields: [Option<IndicatorDataField>; MAX_INDICATOR_EXTRA_FIELDS],
}

impl IndicatorData {
    pub fn scalar(value: f32) -> Self {
        Self {
            value: IndicatorCellValue::Number(value),
            extra_fields: [None; MAX_INDICATOR_EXTRA_FIELDS],
        }
    }

    pub fn boolean(value: bool) -> Self {
        Self {
            value: IndicatorCellValue::Boolean(value),
            extra_fields: [None; MAX_INDICATOR_EXTRA_FIELDS],
        }
    }

    pub fn value(self) -> f32 {
        match self.value {
            IndicatorCellValue::Number(value) => value,
            IndicatorCellValue::Boolean(value) => f32::from(value),
        }
    }

    pub fn primary_value(self) -> IndicatorCellValue {
        self.value
    }

    pub fn with_field(self, key: IndicatorDataFieldKey, value: f32) -> Self {
        self.with_field_value(key, IndicatorCellValue::Number(value))
    }

    pub fn with_bool_field(self, key: IndicatorDataFieldKey, value: bool) -> Self {
        self.with_field_value(key, IndicatorCellValue::Boolean(value))
    }

    fn with_field_value(mut self, key: IndicatorDataFieldKey, value: IndicatorCellValue) -> Self {
        if let Some(slot) = self
            .extra_fields
            .iter_mut()
            .find(|slot| slot.as_ref().map(|field| field.key == key).unwrap_or(false))
        {
            *slot = Some(IndicatorDataField { key, value });
            return self;
        }

        if let Some(slot) = self.extra_fields.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(IndicatorDataField { key, value });
        } else {
            debug_assert!(
                false,
                "IndicatorData extra field capacity exceeded ({MAX_INDICATOR_EXTRA_FIELDS})"
            );
        }

        self
    }

    pub fn field(self, key: IndicatorDataFieldKey) -> Option<f32> {
        match self.field_value(key)? {
            IndicatorCellValue::Number(value) => Some(value),
            IndicatorCellValue::Boolean(_) => None,
        }
    }

    pub fn bool_field(self, key: IndicatorDataFieldKey) -> Option<bool> {
        match self.field_value(key)? {
            IndicatorCellValue::Number(_) => None,
            IndicatorCellValue::Boolean(value) => Some(value),
        }
    }

    pub fn field_value(self, key: IndicatorDataFieldKey) -> Option<IndicatorCellValue> {
        self.extra_fields
            .iter()
            .flatten()
            .find(|field| field.key == key)
            .map(|field| field.value)
    }

    pub fn with_signed_overlay(self, signed_overlay: f32) -> Self {
        self.with_field(IndicatorDataFieldKey::SignedOverlay, signed_overlay)
    }

    pub fn signed_overlay(self) -> Option<f32> {
        self.field(IndicatorDataFieldKey::SignedOverlay)
    }
}
