use super::{
    AvailabilityContext, IndicatorAvailability, IndicatorDefinition, IndicatorDefinitionError,
    IndicatorHistoryRequirement, IndicatorInputRequirement, IndicatorInputSource, IndicatorSetting,
    IndicatorSettingValue, IndicatorSettingsContext, IndicatorUnsupportedReason,
};
use crate::chart::Basis;
use exchange::UnixMs;
use indicator_engine::CompiledIndicator;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::sync::OnceLock;

/// Stable identity for a core package or external filesystem source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IndicatorId {
    Package(String),
    External(String),
}

impl IndicatorId {
    pub fn new(id: impl Into<String>) -> Result<Self, IndicatorIdError> {
        Self::parse(&id.into())
    }

    pub fn external(id: impl Into<String>) -> Result<Self, IndicatorIdError> {
        let id = id.into();
        validate_indicator_id(&id)?;
        if id.starts_with("core.") {
            return Err(IndicatorIdError::ReservedNamespace);
        }
        Ok(Self::External(id))
    }

    pub fn parse(id: &str) -> Result<Self, IndicatorIdError> {
        validate_indicator_id(id)?;
        if id.starts_with("core.") {
            return Ok(Self::Package(id.to_owned()));
        }
        Ok(Self::External(id.to_owned()))
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Package(id) | Self::External(id) => id,
        }
    }

    pub fn as_str(&self) -> &str {
        self.id()
    }

    pub fn supports_market(&self, market: exchange::adapter::MarketKind) -> bool {
        is_supported_for_market(self, market)
    }

    pub fn default_instance_slot(&self) -> u64 {
        self.id()
            .bytes()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                hash.wrapping_mul(0x100000001b3)
                    .wrapping_add(u64::from(byte))
            })
            & 0x0000_7fff_ffff_ffff
    }

    pub fn is_registered(&self) -> bool {
        catalog().get(self).is_some()
    }

    pub fn definition(&self) -> &'static IndicatorDefinition {
        catalog()
            .get(self)
            .map(|registration| &registration.definition)
            .unwrap_or_else(missing_definition)
    }

    pub fn default_config(&self) -> IndicatorConfig {
        catalog()
            .get(self)
            .map(|registration| registration.default_config())
            .unwrap_or_default()
    }

    pub fn normalized_config(&self, config: IndicatorConfig) -> IndicatorConfig {
        catalog()
            .get(self)
            .map(|registration| registration.normalize_config(config.clone()))
            .unwrap_or(config)
    }

    pub fn normalized_config_for_basis(
        &self,
        config: IndicatorConfig,
        _basis: Basis,
    ) -> IndicatorConfig {
        self.normalized_config(config)
    }

    pub fn settings(
        &self,
        config: &IndicatorConfig,
        context: IndicatorSettingsContext,
    ) -> Result<Vec<IndicatorSetting>, IndicatorConfigError> {
        catalog()
            .get(self)
            .ok_or_else(|| IndicatorConfigError::MissingIndicator(self.clone()))
            .and_then(|registration| registration.settings(config.clone(), context))
    }

    pub fn with_setting(
        &self,
        config: IndicatorConfig,
        id: &str,
        value: IndicatorSettingValue,
    ) -> Option<IndicatorConfig> {
        catalog()
            .get(self)
            .and_then(|registration| registration.with_setting(config, id, value))
    }

    pub fn history_requirement(
        &self,
        config: IndicatorConfig,
        basis: Basis,
        reference_time: UnixMs,
    ) -> IndicatorHistoryRequirement {
        catalog()
            .get(self)
            .map(|registration| registration.history_requirement(config, basis, reference_time))
            .unwrap_or_default()
    }

    pub(crate) fn history_requirement_with_warmup(
        &self,
        config: IndicatorConfig,
        basis: Basis,
        reference_time: UnixMs,
        warmup_bars: Option<u64>,
    ) -> IndicatorHistoryRequirement {
        catalog()
            .get(self)
            .map(|registration| {
                registration.history_requirement_with_warmup(
                    config,
                    basis,
                    reference_time,
                    warmup_bars,
                )
            })
            .unwrap_or_default()
    }

    pub fn warmup_bars(&self, config: &IndicatorConfig) -> Option<u64> {
        catalog()
            .get(self)
            .and_then(|registration| registration.warmup_bars(config))
    }

    pub fn unsupported_reason(
        &self,
        context: AvailabilityContext,
    ) -> Option<IndicatorUnsupportedReason> {
        let Some(registration) = catalog().get(self) else {
            return Some(IndicatorUnsupportedReason::NotInstalled);
        };
        if !registration.definition.supported_markets.is_empty()
            && !registration
                .definition
                .supported_markets
                .contains(&context.base_ticker.market_type())
        {
            return Some(IndicatorUnsupportedReason::SourceNotSupported);
        }
        registration.unsupported_reason(context)
    }

    pub fn availability(&self, context: AvailabilityContext) -> IndicatorAvailability {
        if let Some(reason) = self.unsupported_reason(context) {
            return IndicatorAvailability::Unsupported(reason);
        }
        self.definition().availability(context)
    }

    pub fn declared_input_requirements(&self) -> Vec<IndicatorInputRequirement> {
        self.definition().inputs.to_vec()
    }

    pub fn output_channels(&self) -> &'static [super::OutputChannelSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.outputs.as_slice())
            .unwrap_or(&[])
    }

    pub fn default_view(&self) -> &'static super::IndicatorPanelRecipe {
        self.definition().default_view()
    }

    pub fn for_market(market: exchange::adapter::MarketKind) -> Vec<Self> {
        all_indicators()
            .into_iter()
            .filter(|indicator| {
                indicator.definition().supported_markets.is_empty()
                    || indicator.definition().supported_markets.contains(&market)
            })
            .collect()
    }

    pub fn requires_trade_data(&self) -> bool {
        self.definition().requires(IndicatorInputSource::Trades)
    }

    pub fn color_specs(&self) -> &'static [super::IndicatorColorSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.style_slots.as_slice())
            .unwrap_or(&[])
    }

    pub fn mark_specs(&self) -> &'static [super::IndicatorMarkSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.mark_specs.as_slice())
            .unwrap_or(&[])
    }

    pub fn segment_specs(&self) -> &'static [super::IndicatorSegmentSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.segment_specs.as_slice())
            .unwrap_or(&[])
    }

    pub fn region_style_slots(&self) -> &'static [String] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.region_style_slots.as_slice())
            .unwrap_or(&[])
    }

    pub fn mark_source_channel(&self, mark: super::MarkKind) -> Option<&'static str> {
        self.mark_spec(mark)
            .map(|spec| spec.source_channel.as_str())
    }

    pub fn mark_spec(&self, mark: super::MarkKind) -> Option<&'static super::IndicatorMarkSpec> {
        self.mark_specs().iter().find(|spec| spec.mark == mark)
    }

    pub fn fallback_mark_for(&self, mark: super::MarkKind) -> Option<super::MarkKind> {
        self.mark_spec(mark).and_then(|spec| spec.fallback_mark)
    }

    pub fn overlay_channels(&self) -> &'static [super::IndicatorOverlayChannelSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.overlay_channels.as_slice())
            .unwrap_or(&[])
    }

    pub fn readout_specs(&self) -> &'static [super::IndicatorReadoutSpec] {
        match self.default_view() {
            super::IndicatorPanelRecipe::Plot { readouts, .. } => readouts,
            _ => &[],
        }
    }

    pub fn fill_specs(&self) -> &'static [super::IndicatorFillSpec] {
        catalog()
            .get(self)
            .map(|registration| registration.definition.fill_specs.as_slice())
            .unwrap_or(&[])
    }

    pub fn mark_uses_style_slot(&self, mark: super::MarkKind, slot: &str) -> bool {
        self.mark_specs().iter().any(|spec| {
            spec.mark == mark
                && (spec.color_slot == slot
                    || spec.true_color_slot.as_deref() == Some(slot)
                    || spec.false_color_slot.as_deref() == Some(slot)
                    || spec.highlight_color_slot.as_deref() == Some(slot))
        })
    }
}

impl Display for IndicatorId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(registration) = catalog().get(self) {
            return formatter.write_str(&registration.definition.display_name);
        }
        formatter.write_str(self.id())
    }
}

impl Serialize for IndicatorId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for IndicatorId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let id = String::deserialize(deserializer)?;
        Self::parse(&id).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorIdError {
    Empty,
    InvalidCharacter,
    InvalidSegment,
    ReservedNamespace,
}

impl Display for IndicatorIdError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Empty => "indicator ID cannot be empty",
            Self::InvalidCharacter => "indicator ID contains an invalid character",
            Self::InvalidSegment => "indicator ID contains an invalid segment",
            Self::ReservedNamespace => "the core namespace is reserved",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for IndicatorIdError {}

impl std::str::FromStr for IndicatorId {
    type Err = IndicatorIdError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::parse(id)
    }
}

fn validate_indicator_id(id: &str) -> Result<(), IndicatorIdError> {
    if id.is_empty() {
        return Err(IndicatorIdError::Empty);
    }
    if !id.is_ascii() {
        return Err(IndicatorIdError::InvalidCharacter);
    }
    if !id.contains('.') {
        return Err(IndicatorIdError::InvalidSegment);
    }
    for segment in id.split('.') {
        if segment.is_empty()
            || segment.starts_with('-')
            || segment.ends_with('-')
            || segment.bytes().any(|character| {
                !(character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-')
            })
        {
            return Err(IndicatorIdError::InvalidSegment);
        }
    }
    Ok(())
}

/// Serialized configuration owned by an indicator instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IndicatorConfig(serde_json::Value);

impl Default for IndicatorConfig {
    fn default() -> Self {
        Self(serde_json::Value::Object(serde_json::Map::new()))
    }
}

impl IndicatorConfig {
    pub fn default_for(indicator: &IndicatorId) -> Self {
        indicator.default_config()
    }

    pub fn from_typed<T: Serialize>(config: &T) -> Result<Self, IndicatorConfigError> {
        serde_json::to_value(config)
            .map(Self)
            .map_err(|error| IndicatorConfigError::Serialize(error.to_string()))
    }

    pub fn from_value(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub fn into_value(self) -> serde_json::Value {
        self.0
    }

    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    pub fn is_valid_for(&self, indicator: &IndicatorId) -> bool {
        catalog()
            .get(indicator)
            .is_some_and(|registration| registration.validate_config(self).is_ok())
    }

    pub fn normalized_for(self, indicator: &IndicatorId) -> Self {
        indicator.normalized_config(self)
    }

    pub fn normalized_for_basis(self, indicator: &IndicatorId, basis: Basis) -> Self {
        indicator.normalized_config_for_basis(self, basis)
    }

    pub fn settings_for(
        &self,
        indicator: &IndicatorId,
        context: IndicatorSettingsContext,
    ) -> Result<Vec<IndicatorSetting>, IndicatorConfigError> {
        indicator.settings(self, context)
    }

    pub fn with_setting_for(
        self,
        indicator: &IndicatorId,
        id: &str,
        value: IndicatorSettingValue,
    ) -> Option<Self> {
        indicator.with_setting(self, id, value)
    }

    pub fn history_requirement_for(
        self,
        indicator: &IndicatorId,
        basis: Basis,
        reference_time: UnixMs,
    ) -> IndicatorHistoryRequirement {
        indicator.history_requirement(self, basis, reference_time)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndicatorConfigError {
    MissingIndicator(IndicatorId),
    Deserialize(String),
    Serialize(String),
    InvalidDefinition(IndicatorDefinitionError),
}

impl Display for IndicatorConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingIndicator(id) => {
                write!(formatter, "indicator {} is not installed", id.id())
            }
            Self::Deserialize(error) => {
                write!(formatter, "invalid indicator configuration: {error}")
            }
            Self::Serialize(error) => write!(
                formatter,
                "could not serialize indicator configuration: {error}"
            ),
            Self::InvalidDefinition(error) => {
                write!(formatter, "invalid indicator definition: {error:?}")
            }
        }
    }
}

impl std::error::Error for IndicatorConfigError {}

#[derive(Clone)]
pub struct IndicatorRegistration {
    /// Stable source ID used in persistence and catalog lookup.
    pub id: IndicatorId,
    /// Renderer and feed metadata shared by every instance of the indicator.
    pub definition: IndicatorDefinition,
    pub(super) compiled: Arc<CompiledIndicator>,
}

impl std::fmt::Debug for IndicatorRegistration {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndicatorRegistration")
            .field("id", &self.id)
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

impl IndicatorRegistration {
    pub(super) fn from_parts(
        id: IndicatorId,
        definition: IndicatorDefinition,
        compiled: Arc<CompiledIndicator>,
    ) -> Self {
        Self {
            id,
            definition,
            compiled,
        }
    }
}

// Indicators are discovered once while the application starts. Keeping one
// immutable process-wide catalog makes source edits deterministic: a restart
// is the explicit boundary at which new source is activated.
static CATALOG: OnceLock<IndicatorCatalog> = OnceLock::new();
// Library callers can inspect an ID before the host has completed startup
// (for example while restoring persisted state). Keep that read harmless
// without initializing the real OnceLock; doing so must not turn a later
// startup installation into a spurious second initialization.
static EMPTY_CATALOG: std::sync::LazyLock<IndicatorCatalog> =
    std::sync::LazyLock::new(|| IndicatorCatalog::from_registrations(Vec::new()));

pub fn catalog() -> &'static IndicatorCatalog {
    CATALOG.get().unwrap_or(&EMPTY_CATALOG)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogInstallError {
    AlreadyInitialized,
    Diagnostics(Vec<String>),
}

impl Display for CatalogInstallError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInitialized => {
                formatter.write_str("indicator catalog is already initialized")
            }
            Self::Diagnostics(diagnostics) => write!(formatter, "{}", diagnostics.join("\n")),
        }
    }
}

impl std::error::Error for CatalogInstallError {}

/// Install a catalog while retaining source-loader diagnostics alongside
/// validation diagnostics. A malformed installed source is intentionally not
/// fatal: valid registrations are installed and the rejected source remains
/// visible through `IndicatorCatalog::diagnostics`.
pub(crate) fn install_catalog_with_core_diagnostics(
    core_registrations: Vec<IndicatorRegistration>,
    installed_registrations: Vec<IndicatorRegistration>,
    loader_diagnostics: Vec<String>,
) -> Result<&'static IndicatorCatalog, CatalogInstallError> {
    let (registrations, loader_diagnostics) = merge_core_and_installed(
        core_registrations,
        installed_registrations,
        loader_diagnostics,
    );
    let catalog =
        IndicatorCatalog::from_registrations_with_diagnostics(registrations, loader_diagnostics);
    for diagnostic in &catalog.diagnostics {
        log::warn!("indicator catalog: quarantined source: {diagnostic}");
    }

    CATALOG
        .set(catalog)
        .map_err(|_| CatalogInstallError::AlreadyInitialized)?;
    Ok(CATALOG.get().expect("catalog was installed"))
}

fn merge_core_and_installed(
    core_registrations: Vec<IndicatorRegistration>,
    installed_registrations: Vec<IndicatorRegistration>,
    mut diagnostics: Vec<String>,
) -> (Vec<IndicatorRegistration>, Vec<String>) {
    let mut registrations = core_registrations;
    let core_ids = registrations
        .iter()
        .map(|registration| registration.id.clone())
        .collect::<FxHashSet<_>>();
    let mut seen_ids = core_ids.clone();

    for registration in installed_registrations {
        let id = registration.id.clone();
        if !seen_ids.insert(id.clone()) {
            let reason = if core_ids.contains(&id) {
                "it is provided by the core catalog"
            } else {
                "an earlier installed source already provides it"
            };
            diagnostics.push(format!(
                "installed indicator '{}' ignored because {reason}",
                id.id()
            ));
            log::warn!(
                "indicator catalog: ignoring later installed source '{}'",
                id.id()
            );
            continue;
        }
        registrations.push(registration);
    }

    (registrations, diagnostics)
}

#[derive(Debug)]
pub struct IndicatorCatalog {
    registrations: Vec<IndicatorRegistration>,
    by_id: FxHashMap<IndicatorId, usize>,
    diagnostics: Vec<String>,
}

impl IndicatorCatalog {
    /// Build an isolated catalog for validation or integration tests.
    pub(crate) fn from_registrations(registrations: Vec<IndicatorRegistration>) -> Self {
        Self::from_registrations_with_diagnostics(registrations, Vec::new())
    }

    pub(crate) fn from_registrations_with_diagnostics(
        mut registrations: Vec<IndicatorRegistration>,
        mut diagnostics: Vec<String>,
    ) -> Self {
        registrations.sort_by(|left, right| left.id.id().cmp(right.id.id()));

        let mut valid = Vec::with_capacity(registrations.len());
        let mut by_id = FxHashMap::default();
        for registration in registrations {
            if let Err(error) = IndicatorId::parse(registration.id.id()) {
                diagnostics.push(format!(
                    "{}: invalid indicator ID: {error}",
                    registration.id.id()
                ));
                continue;
            }
            if matches!(registration.id, IndicatorId::External(_))
                && registration.id.id().starts_with("core.")
            {
                diagnostics.push(format!(
                    "{}: externally tagged registrations may not use the core namespace",
                    registration.id.id()
                ));
                continue;
            }
            let id = registration.id.clone();
            if by_id.contains_key(&id) {
                diagnostics.push(format!("duplicate indicator ID: {}", registration.id.id()));
                continue;
            }
            let default_config = registration.default_config();
            if let Err(error) = registration.validate_config(&default_config) {
                diagnostics.push(format!(
                    "{}: invalid default configuration: {error}",
                    registration.id.id()
                ));
                continue;
            }
            let index = valid.len();
            valid.push(registration);
            by_id.insert(id, index);
        }
        for diagnostic in &diagnostics {
            log::error!("indicator catalog: {diagnostic}");
        }
        Self {
            registrations: valid,
            by_id,
            diagnostics,
        }
    }

    pub fn registrations(&self) -> impl ExactSizeIterator<Item = &IndicatorRegistration> + '_ {
        self.registrations.iter()
    }

    pub fn ids(&self) -> impl ExactSizeIterator<Item = IndicatorId> + '_ {
        self.registrations
            .iter()
            .map(|registration| registration.id.clone())
    }

    pub fn get(&self, id: &IndicatorId) -> Option<&IndicatorRegistration> {
        self.by_id
            .get(id)
            .and_then(|index| self.registrations.get(*index))
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

pub fn all_indicators() -> Vec<IndicatorId> {
    catalog().ids().collect()
}

pub fn indicator_definition(indicator: &IndicatorId) -> &'static IndicatorDefinition {
    indicator.definition()
}

pub fn is_supported_for_market(
    indicator: &IndicatorId,
    market: exchange::adapter::MarketKind,
) -> bool {
    indicator.definition().supported_markets.is_empty()
        || indicator.definition().supported_markets.contains(&market)
}

pub fn declared_input_requirements(indicator: &IndicatorId) -> Vec<IndicatorInputRequirement> {
    indicator.declared_input_requirements()
}

pub fn display_name(indicator: &IndicatorId) -> &'static str {
    &indicator.definition().display_name
}

pub fn panel_recipe(indicator: &IndicatorId) -> &'static super::IndicatorPanelRecipe {
    indicator.default_view()
}

pub fn requires_trade_data(indicator: &IndicatorId) -> bool {
    indicator.requires_trade_data()
}

pub fn table_columns(indicator: &IndicatorId) -> &'static [super::TableColumnSpec] {
    match &indicator.definition().panel_recipe {
        super::IndicatorPanelRecipe::Table { columns, .. } => columns,
        _ => &[],
    }
}

pub fn unsupported_reason(
    indicator: &IndicatorId,
    context: AvailabilityContext,
) -> Option<IndicatorUnsupportedReason> {
    indicator.unsupported_reason(context)
}

pub fn availability(
    indicator: &IndicatorId,
    context: AvailabilityContext,
) -> IndicatorAvailability {
    indicator.availability(context)
}

fn missing_definition() -> &'static IndicatorDefinition {
    static MISSING_DEFINITION: OnceLock<IndicatorDefinition> = OnceLock::new();
    MISSING_DEFINITION.get_or_init(|| {
        IndicatorDefinition::new(
            "Unavailable indicator",
            Vec::new(),
            Vec::new(),
            super::IndicatorPanelRecipe::Plot {
                panel_title: "Unavailable indicator".to_owned(),
                layer_name: "Unavailable indicator".to_owned(),
                value_precision: super::PanelValuePrecision::FixedStep(1.0),
                value_label_policy: super::PanelValueLabelPolicy {
                    axis_mode: super::super::composition::PanelValueLabelMode::Compact,
                    header_mode: super::super::composition::PanelValueLabelMode::Commas,
                    max_decimals: None,
                },
                preferred_scale: super::PanelScaleMode::FitVisible,
                layers: Vec::new(),
                readouts: Vec::new(),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_ids_are_stable_and_catalog_order_is_deterministic() {
        assert!(IndicatorId::parse("core.volume").is_ok());
        let catalog_ids: Vec<_> = all_indicators()
            .into_iter()
            .map(|id| id.id().to_owned())
            .collect();
        assert!(catalog_ids.windows(2).all(|window| window[0] < window[1]));
    }

    #[test]
    fn ids_validate_external_and_reserved_namespaces() {
        assert!(IndicatorId::external("community.ema-cross").is_ok());
        assert!(IndicatorId::external("ema-cross").is_err());
        assert!(IndicatorId::external("Community.EMA").is_err());
        assert!(IndicatorId::external("core.custom").is_err());
        assert!(IndicatorId::parse("catalog.rsi").is_ok());
        assert_eq!(
            IndicatorId::parse("core.future"),
            Ok(IndicatorId::Package("core.future".to_owned()))
        );
    }

    #[test]
    fn invalid_and_duplicate_registrations_are_excluded() {
        let scripted = super::super::language::repository_registrations()
            .into_iter()
            .find(|registration| registration.id.id() == "core.rsi")
            .expect("repository RSI registration");
        let mut invalid = scripted.clone();
        invalid.id = IndicatorId::External("core.future".to_owned());
        let registrations = vec![invalid, scripted.clone(), scripted];
        let catalog = IndicatorCatalog::from_registrations(registrations);
        assert_eq!(catalog.registrations().len(), 1);
        assert!(
            catalog
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.contains("namespace"))
        );
        assert!(
            catalog
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.contains("duplicate"))
        );
    }

    #[test]
    fn core_wins_and_installed_sources_keep_first_registration() {
        let registrations = super::super::language::repository_registrations();
        let core = registrations
            .iter()
            .find(|registration| registration.id.id() == "core.volume")
            .expect("repository Volume registration")
            .clone();
        let rsi = registrations
            .iter()
            .find(|registration| registration.id.id() == "core.rsi")
            .expect("repository RSI registration");
        let mut first_installed = rsi.clone();
        first_installed.definition = IndicatorDefinition {
            display_name: "First installed registration".to_owned(),
            ..first_installed.definition
        };
        let mut later_installed = rsi.clone();
        later_installed.definition = IndicatorDefinition {
            display_name: "Later installed registration".to_owned(),
            ..later_installed.definition
        };

        let (merged, diagnostics) = merge_core_and_installed(
            vec![core.clone()],
            vec![core, first_installed, later_installed],
            Vec::new(),
        );
        let catalog = IndicatorCatalog::from_registrations_with_diagnostics(merged, diagnostics);

        assert_eq!(
            catalog
                .get(&IndicatorId::parse("core.volume").unwrap())
                .expect("core Volume should be registered")
                .definition
                .display_name,
            "Volume"
        );
        assert_eq!(
            catalog
                .get(&IndicatorId::parse("core.rsi").unwrap())
                .expect("first installed RSI should be registered")
                .definition
                .display_name,
            "First installed registration"
        );
        assert!(
            catalog
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.contains("provided by the core catalog"))
        );
        assert!(
            catalog
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic
                    .contains("earlier installed source already provides it"))
        );
    }
}
