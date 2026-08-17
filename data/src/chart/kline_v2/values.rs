#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorDataFieldKey {
    SignedOverlay,
    Custom(&'static str),
}

pub const BOLLINGER_UPPER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.upper");
pub const BOLLINGER_LOWER_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("bollinger.lower");
pub const RSI_SIGNAL_FIELD_KEY: IndicatorDataFieldKey = IndicatorDataFieldKey::Custom("rsi.signal");
pub const RSI_UPPER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.upper_band");
pub const RSI_LOWER_BAND_FIELD_KEY: IndicatorDataFieldKey =
    IndicatorDataFieldKey::Custom("rsi.lower_band");

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorDataField {
    pub key: IndicatorDataFieldKey,
    pub value: f32,
}

const MAX_INDICATOR_EXTRA_FIELDS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorData {
    value: f32,
    extra_fields: [Option<IndicatorDataField>; MAX_INDICATOR_EXTRA_FIELDS],
}

impl IndicatorData {
    pub fn scalar(value: f32) -> Self {
        Self {
            value,
            extra_fields: [None; MAX_INDICATOR_EXTRA_FIELDS],
        }
    }

    pub fn value(self) -> f32 {
        self.value
    }

    pub fn with_field(mut self, key: IndicatorDataFieldKey, value: f32) -> Self {
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
