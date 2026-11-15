/// A setting that only takes effect on the next start.
///
/// `applied` is what the running process uses and never changes. `wanted` is
/// what gets persisted and used after a restart. The value passed to `new` must
/// also be what any process-wide projection is seeded with (`SIZE_CALC_UNIT`,
/// `TRADE_FETCH_MODE`, `iced::Settings::antialiasing`), so both come from the
/// same `data::State` field at boot and cannot disagree.
pub struct Deferred<T> {
    applied: T,
    wanted: T,
}

impl<T: Clone> Deferred<T> {
    pub fn new(value: T) -> Self {
        Self {
            applied: value.clone(),
            wanted: value,
        }
    }

    pub fn applied(&self) -> &T {
        &self.applied
    }

    pub fn wanted(&self) -> &T {
        &self.wanted
    }

    pub fn set_wanted(&mut self, value: T) {
        self.wanted = value;
    }
}

impl<T> Deferred<T> {
    /// Whether one field differs between the running and wanted values.
    ///
    /// Use this when `T` bundles settings that each take effect on their own, so
    /// one setting's marker never claims another one needs a restart.
    pub fn differs<U: PartialEq>(&self, field: impl Fn(&T) -> &U) -> bool {
        field(&self.wanted) != field(&self.applied)
    }
}

impl<T: PartialEq> Deferred<T> {
    /// Whether the value as a whole differs. Use this when the setting is its
    /// own type rather than a bundle of independent ones.
    pub fn restart_required(&self) -> bool {
        self.wanted != self.applied
    }
}
