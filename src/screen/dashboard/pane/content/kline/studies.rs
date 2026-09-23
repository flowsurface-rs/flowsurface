use crate::widget::study_configurator as study;
use data::chart::Basis;
use data::chart::kline::{KlineStudy, KlineStudyId, SourceConstraint, StudyDataSource};

pub enum StudyCommand {
    Set(Vec<KlineStudy>),
    Toggle(KlineStudy),
    ToggleVisibility(KlineStudyId),
    ToggleAllVisibility,
    ToggleExpansion,
    Remove(KlineStudyId),
    RemoveAll,
}

#[derive(Clone, Copy)]
pub(super) enum StudyChange {
    Unchanged,
    Changed,
}

impl StudyChange {
    pub(super) const fn changed(self) -> bool {
        matches!(self, Self::Changed)
    }
}

#[derive(Clone)]
pub(super) struct StudySnapshot {
    active: Vec<KlineStudy>,
    configs: Vec<KlineStudy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StudyAvailability {
    Available,
    UnsupportedBasis,
    RequiresTradeData,
    SourceConflict(StudyDataSource),
}

impl StudySnapshot {
    pub(super) fn new(active: Vec<KlineStudy>, configs: Vec<KlineStudy>) -> Self {
        Self { active, configs }
    }
}

pub(super) struct StudyState {
    active: Vec<KlineStudy>,
    configs: Vec<KlineStudy>,
    configurator: study::Configurator<KlineStudy>,
    legend_expanded: bool,
}

impl Default for StudyState {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            configs: Vec::new(),
            configurator: study::Configurator::new(),
            legend_expanded: true,
        }
    }
}

impl StudyState {
    pub(super) fn active(&self) -> &[KlineStudy] {
        &self.active
    }

    pub(super) fn active_snapshot(&self) -> Vec<KlineStudy> {
        self.active.clone()
    }

    pub(super) fn configs_snapshot(&self) -> Vec<KlineStudy> {
        self.configs.clone()
    }

    pub(super) fn legend_expanded(&self) -> bool {
        self.legend_expanded
    }

    pub(super) fn study(&self, id: KlineStudyId) -> Option<KlineStudy> {
        self.active.iter().copied().find(|study| study.id() == id)
    }

    pub(super) fn availability(
        &self,
        study: KlineStudy,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> StudyAvailability {
        if study.normalized_for_basis(basis).is_none() {
            return StudyAvailability::UnsupportedBasis;
        }

        if let SourceConstraint::Exclusive(source) = study.source_constraint()
            && let Some(conflicting) =
                self.active
                    .iter()
                    .find_map(|active| match active.source_constraint() {
                        SourceConstraint::Exclusive(other) if other != source => Some(other),
                        SourceConstraint::Exclusive(_) | SourceConstraint::Independent => None,
                    })
        {
            return StudyAvailability::SourceConflict(conflicting);
        }

        if !trade_data_enabled && study.requires_trade_data() {
            return StudyAvailability::RequiresTradeData;
        }

        StudyAvailability::Available
    }

    pub(super) fn restore(&mut self, snapshot: StudySnapshot) {
        self.active = snapshot.active;
        self.configs = snapshot.configs;
    }

    pub(super) fn normalize_for_basis(&mut self, basis: Basis) {
        self.active = Self::normalized_active(self.active.iter().copied(), basis);
        self.configs = Self::normalized_for_basis(self.configs.iter().copied(), basis);
        self.remember_configs(&self.active.clone());
    }

    pub(super) fn apply(
        &mut self,
        command: StudyCommand,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> StudyChange {
        match command {
            StudyCommand::Set(studies) => self.apply_set(studies, basis),
            StudyCommand::Toggle(study) => {
                let is_active = self.active.iter().any(|active| active.is_same_type(study));
                let Some(studies) =
                    self.studies_with_active(study, !is_active, basis, trade_data_enabled)
                else {
                    return StudyChange::Unchanged;
                };
                self.apply_set(studies, basis)
            }
            StudyCommand::ToggleVisibility(id) => {
                let Some(index) = self.active.iter().position(|study| study.id() == id) else {
                    return StudyChange::Unchanged;
                };
                let study = self.active[index];
                self.active[index] = study.with_visibility(!study.visible());
                self.remember_configs(&self.active.clone());
                StudyChange::Changed
            }
            StudyCommand::ToggleAllVisibility => {
                let should_show = !self.active.iter().all(|study| study.visible());
                let mut changed = false;
                for study in &mut self.active {
                    if study.visible() != should_show {
                        *study = study.with_visibility(should_show);
                        changed = true;
                    }
                }
                if changed {
                    self.remember_configs(&self.active.clone());
                }
                if changed {
                    StudyChange::Changed
                } else {
                    StudyChange::Unchanged
                }
            }
            StudyCommand::ToggleExpansion => {
                self.legend_expanded = !self.legend_expanded;
                StudyChange::Changed
            }
            StudyCommand::Remove(id) => {
                let original_len = self.active.len();
                self.active.retain(|study| study.id() != id);
                let changed = self.active.len() != original_len;
                if changed {
                    StudyChange::Changed
                } else {
                    StudyChange::Unchanged
                }
            }
            StudyCommand::RemoveAll => {
                let changed = !self.active.is_empty();
                self.active.clear();
                if changed {
                    StudyChange::Changed
                } else {
                    StudyChange::Unchanged
                }
            }
        }
    }

    pub(super) fn update_configurator(
        &mut self,
        message: study::Message<KlineStudy>,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> Option<StudyCommand> {
        let action = self.configurator.update(message)?;
        match action {
            study::Action::ToggleStudy(study, is_active) => self
                .studies_with_active(study, is_active, basis, trade_data_enabled)
                .map(StudyCommand::Set),
            study::Action::ConfigureStudy(study) => {
                let mut studies = self.active.clone();
                let active = studies
                    .iter_mut()
                    .find(|active| active.is_same_type(study))?;
                *active = study;
                Some(StudyCommand::Set(studies))
            }
        }
    }

    fn apply_set(&mut self, studies: Vec<KlineStudy>, basis: Basis) -> StudyChange {
        let normalized = Self::normalized_active(studies, basis);
        self.remember_configs(&normalized);
        let changed = self.active != normalized;
        if changed {
            self.active = normalized;
        }
        if changed {
            StudyChange::Changed
        } else {
            StudyChange::Unchanged
        }
    }

    fn studies_with_active(
        &self,
        study: KlineStudy,
        is_active: bool,
        basis: Basis,
        trade_data_enabled: bool,
    ) -> Option<Vec<KlineStudy>> {
        if is_active
            && self.availability(study, basis, trade_data_enabled) != StudyAvailability::Available
        {
            return None;
        }

        let mut studies = self.active.clone();
        if is_active {
            let configured = self
                .configs
                .iter()
                .find(|saved| saved.is_same_type(study))
                .copied()
                .unwrap_or(study)
                .with_visibility(true);
            if studies.iter().any(|active| active.is_same_type(configured)) {
                return None;
            }
            studies.retain(|active| !active.same_family(configured));
            studies.push(configured);
        } else {
            let original_len = studies.len();
            studies.retain(|active| !active.is_same_type(study));
            if studies.len() == original_len {
                return None;
            }
        }
        Some(studies)
    }

    fn remember_configs(&mut self, studies: &[KlineStudy]) {
        for study in studies {
            if let Some(configured) = self
                .configs
                .iter_mut()
                .find(|configured| configured.is_same_type(*study))
            {
                *configured = *study;
            } else {
                self.configs.push(*study);
            }
        }
    }

    fn normalized_active(
        studies: impl IntoIterator<Item = KlineStudy>,
        basis: Basis,
    ) -> Vec<KlineStudy> {
        let studies = studies.into_iter().collect::<Vec<_>>();
        let source = studies
            .iter()
            .rev()
            .find_map(|study| match study.source_constraint() {
                SourceConstraint::Exclusive(source) => Some(source),
                SourceConstraint::Independent => None,
            });
        let studies = studies
            .into_iter()
            .filter(|study| match study.source_constraint() {
                SourceConstraint::Exclusive(study_source) => Some(study_source) == source,
                SourceConstraint::Independent => true,
            });

        Self::active_for_families(Self::normalized_for_basis(studies, basis))
    }

    fn normalized_for_basis(
        studies: impl IntoIterator<Item = KlineStudy>,
        basis: Basis,
    ) -> Vec<KlineStudy> {
        let mut normalized = Vec::new();
        for study in studies {
            let Some(study) = study.normalized_for_basis(basis) else {
                continue;
            };
            if !normalized
                .iter()
                .any(|existing: &KlineStudy| existing.is_same_type(study))
            {
                normalized.push(study);
            }
        }
        normalized
    }

    fn active_for_families(studies: Vec<KlineStudy>) -> Vec<KlineStudy> {
        let mut active: Vec<KlineStudy> = Vec::new();
        for study in studies {
            active.retain(|existing| !existing.same_family(study));
            active.push(study);
        }
        active
    }
}
