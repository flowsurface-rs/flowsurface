use crate::chart::kline::{ChartPreset, SettingsSnapshot};
use crate::layout::MAX_NAME_LENGTH;
use uuid::Uuid;

#[derive(Default)]
pub struct Library {
    presets: Vec<ChartPreset>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Create {
        name: String,
        settings: Box<SettingsSnapshot>,
    },
    Save {
        id: Uuid,
        settings: Box<SettingsSnapshot>,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    Duplicate(Uuid),
    Remove(Uuid),
    Reorder {
        index: usize,
        target: usize,
    },
}

impl Library {
    pub fn from_config(presets: Vec<ChartPreset>) -> Self {
        Self {
            presets: presets
                .into_iter()
                .map(|mut preset| {
                    preset.settings = preset.settings.normalized_preset();
                    preset
                })
                .collect(),
        }
    }

    pub fn entries(&self) -> &[ChartPreset] {
        &self.presets
    }

    pub fn get(&self, id: Option<Uuid>) -> Option<&ChartPreset> {
        self.presets.iter().find(|preset| Some(preset.id) == id)
    }

    pub fn unique_name(&self, proposed: &str, except: Option<Uuid>) -> String {
        crate::util::unique_name(
            if proposed.trim().is_empty() {
                "Preset"
            } else {
                proposed
            },
            self.presets
                .iter()
                .filter(|preset| Some(preset.id) != except)
                .map(|preset| preset.name.as_str()),
            MAX_NAME_LENGTH,
        )
    }

    fn get_mut(&mut self, id: Uuid) -> Result<&mut ChartPreset, String> {
        self.presets
            .iter_mut()
            .find(|preset| preset.id == id)
            .ok_or_else(|| "This preset no longer exists.".to_owned())
    }

    fn add(&mut self, name: &str, settings: SettingsSnapshot) -> Uuid {
        let id = Uuid::new_v4();
        self.presets.push(ChartPreset {
            id,
            name: self.unique_name(name, None),
            settings: settings.normalized_preset(),
        });
        id
    }

    pub fn update(&mut self, command: Command) -> Result<Option<Uuid>, String> {
        match command {
            Command::Create { name, settings } => {
                if name.trim().is_empty() {
                    return Err("Enter a preset name.".into());
                }
                Ok(Some(self.add(&name, *settings)))
            }
            Command::Save { id, settings } => {
                self.get_mut(id)?.settings = settings.normalized_preset();
                Ok(None)
            }
            Command::Rename { id, name } => {
                if name.trim().is_empty() {
                    return Err("Enter a preset name.".into());
                }
                let name = self.unique_name(&name, Some(id));
                self.get_mut(id)?.name = name;
                Ok(None)
            }
            Command::Duplicate(id) => {
                let preset = self.get(Some(id)).ok_or("This preset no longer exists.")?;
                let name = format!("{} copy", preset.name);
                self.add(&name, preset.settings.clone());
                Ok(None)
            }
            Command::Remove(id) => {
                self.get_mut(id)?;
                self.presets.retain(|preset| preset.id != id);
                Ok(None)
            }
            Command::Reorder { index, target } => {
                crate::util::reorder_vec(&mut self.presets, index, target)
                    .ok_or("This preset order is no longer available.")?;
                Ok(None)
            }
        }
    }
}
