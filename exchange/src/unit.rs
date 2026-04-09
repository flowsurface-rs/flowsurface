pub mod price;
pub mod qty;

pub use price::{Price, PriceStep};
pub use qty::Qty;

pub type ContractSize = Power10<-4, 6>;
pub type MinTicksize = Power10<-8, 2>;
pub type MinQtySize = Power10<-6, 8>;

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq)]
pub struct Power10<const MIN: i8, const MAX: i8> {
    pub power: i8,
}

impl<const MIN: i8, const MAX: i8> Power10<MIN, MAX> {
    #[inline]
    pub fn new(power: i8) -> Self {
        Self {
            power: power.clamp(MIN, MAX),
        }
    }

    #[inline]
    pub fn as_f32(self) -> f32 {
        10f32.powi(self.power as i32)
    }
}

impl<const MIN: i8, const MAX: i8> From<Power10<MIN, MAX>> for f32 {
    fn from(v: Power10<MIN, MAX>) -> Self {
        v.as_f32()
    }
}

impl<const MIN: i8, const MAX: i8> From<f32> for Power10<MIN, MAX> {
    fn from(value: f32) -> Self {
        if value <= 0.0 {
            return Self { power: 0 };
        }
        let log10 = value.abs().log10();
        let rounded = log10.round() as i8;
        let power = rounded.clamp(MIN, MAX);
        Self { power }
    }
}

impl<const MIN: i8, const MAX: i8> serde::Serialize for Power10<MIN, MAX> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // serialize as a plain numeric (e.g. 0.1, 1, 10)
        let v: f32 = (*self).into();
        serializer.serialize_f32(v)
    }
}

impl<'de, const MIN: i8, const MAX: i8> serde::Deserialize<'de> for Power10<MIN, MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = f32::deserialize(deserializer)?;
        Ok(Self::from(v))
    }
}
