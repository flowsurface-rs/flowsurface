use std::marker::PhantomData;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Abs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rel;

/// A bucket index in either absolute or relative space (ms/aggr_time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bucket<S>(pub i64, PhantomData<S>);

impl Bucket<Abs> {
    #[inline]
    pub const fn abs(v: i64) -> Self {
        Self(v, PhantomData)
    }

    #[inline]
    pub const fn to_rel(self, ref_bucket: Bucket<Abs>) -> Bucket<Rel> {
        Bucket::<Rel>::rel(self.0 - ref_bucket.0)
    }
}

impl Bucket<Rel> {
    #[inline]
    pub const fn rel(v: i64) -> Self {
        Self(v, PhantomData)
    }
}

/// A y-bin index in either absolute or relative space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YBin<S>(pub i64, PhantomData<S>);

impl YBin<Abs> {
    #[inline]
    pub const fn abs(v: i64) -> Self {
        Self(v, PhantomData)
    }

    #[inline]
    pub const fn to_rel(self, base_abs: YBin<Abs>) -> YBin<Rel> {
        YBin::<Rel>::rel(self.0 - base_abs.0)
    }
}

impl YBin<Rel> {
    #[inline]
    pub const fn rel(v: i64) -> Self {
        Self(v, PhantomData)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpanEnd {
    /// Exclusive end bucket (absolute bucket space).
    Closed(Bucket<Abs>),
    /// Open-ended “until now”.
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BucketSpan {
    pub start: Bucket<Abs>,
    pub end_excl: SpanEnd,
}
