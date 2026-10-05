#[derive(Clone, Copy)]
pub(crate) enum ConfigEdit<'a> {
    Theme(&'a str),
    StatusIndicators(super::StatusIndicatorStyle),
    Sound(bool),
    ToastDelivery(super::ToastDelivery),
    AgentTree(bool),
}

impl ConfigEdit<'_> {
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Theme(_) => "theme",
            Self::StatusIndicators(_) => "status indicators",
            Self::Sound(_) => "sound setting",
            Self::ToastDelivery(_) => "toast setting",
            Self::AgentTree(_) => "agent tree setting",
        }
    }

    pub(crate) fn apply(self, content: &str) -> String {
        match self {
            Self::Theme(name) => {
                let content =
                    super::upsert_section_value(content, "theme", "name", &format!("\"{name}\""));
                super::upsert_section_bool(&content, "theme", "auto_switch", false)
            }
            Self::StatusIndicators(style) => super::upsert_section_value(
                content,
                "ui",
                "status_indicators",
                &format!("\"{}\"", style.as_str()),
            ),
            Self::AgentTree(enabled) => {
                super::upsert_section_bool(content, "ui.sidebar.agents", "tree", enabled)
            }
            Self::Sound(enabled) => {
                super::upsert_section_bool(content, "ui.sound", "enabled", enabled)
            }
            Self::ToastDelivery(delivery) => {
                let value = match delivery {
                    super::ToastDelivery::Off => "\"off\"",
                    super::ToastDelivery::Herdr => "\"herdr\"",
                    super::ToastDelivery::Terminal => "\"terminal\"",
                    super::ToastDelivery::System => "\"system\"",
                };
                let content = super::upsert_section_value(content, "ui.toast", "delivery", value);
                super::remove_section_key(&content, "ui.toast", "enabled")
            }
        }
    }

    fn checked_apply(self, content: &str) -> Result<String, String> {
        let updated = self.apply(content);
        if let Self::AgentTree(enabled) = self {
            // The line-based writer cannot edit every valid TOML representation.
            // Check the entire parsed document before writing, including unknown keys.
            let mut expected: toml::Value = toml::from_str(content)
                .map_err(|error| format!("cannot edit agent tree config: {error}"))?;
            let mut table = expected.as_table_mut().ok_or("config must be a table")?;
            for key in ["ui", "sidebar", "agents"] {
                table = table
                    .entry(key)
                    .or_insert_with(|| toml::Value::Table(Default::default()))
                    .as_table_mut()
                    .ok_or("agent tree config must use tables")?;
            }
            table.insert("tree".into(), toml::Value::Boolean(enabled));
            let guidance = format!(
                "cannot safely edit agent tree config; manually set tree = {enabled} under [ui.sidebar.agents] (outside any arrays or inline tables)"
            );
            let actual = toml::from_str::<toml::Value>(&updated).map_err(|_| guidance.clone())?;
            if actual != expected {
                return Err(guidance);
            }
        }
        Ok(updated)
    }
}

pub(crate) fn update_file_at(
    path: &std::path::Path,
    description: &str,
    update: impl FnOnce(&str) -> String,
) -> Result<(), String> {
    update_file_at_checked(path, description, |content| Ok(update(content)))
}

fn update_file_at_checked(
    path: &std::path::Path,
    description: &str,
    update: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }
    let content = match super::io::read_optional_config(path) {
        Ok(Some(content)) => content,
        Ok(None) => String::new(),
        Err(error) => {
            return Err(format!(
                "failed to read config before saving {description}: {error}"
            ));
        }
    };
    let updated = update(&content)?;
    std::fs::write(path, updated).map_err(|error| format!("failed to save {description}: {error}"))
}

pub(crate) fn write_edit(edit: ConfigEdit<'_>) -> Result<(), String> {
    update_file_at_checked(&super::config_path(), edit.description(), |content| {
        edit.checked_apply(content)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tree_edit_preserves_layouts_and_rejects_unsafe_toml_without_writing() {
        let dir =
            std::env::temp_dir().join(format!("herdr-agent-tree-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        for content in [
            "",
            "[ui.sidebar.agents]\nrows = [[\"agent\"]]\nrow_gap = 2\n[ui.sidebar.agents.rows_by_agent]\npi = [[\"tab\"]]\n[other]\nkeep = 7\n",
            "[ui.sidebar.agents]\ntree = false\nrows = [[\"agent\"]]\n",
            "[ui.sidebar.agents]\ntree=false # comment\nrows = [\n [\"agent\"],\n]\n",
        ] {
            std::fs::write(&path, content).unwrap();
            for enabled in [true, false] {
                update_file_at_checked(&path, "agent tree", |content| {
                    ConfigEdit::AgentTree(enabled).checked_apply(content)
                }).unwrap();
                let written = std::fs::read_to_string(&path).unwrap();
                let parsed: toml::Value = toml::from_str(&written).unwrap();
                assert_eq!(parsed["ui"]["sidebar"]["agents"]["tree"].as_bool(), Some(enabled));
                let mut actual = parsed;
                actual["ui"]["sidebar"]["agents"].as_table_mut().unwrap().remove("tree");
                if !content.is_empty() {
                    let mut expected: toml::Value = toml::from_str(content).unwrap();
                    expected["ui"]["sidebar"]["agents"].as_table_mut().unwrap().remove("tree");
                    assert_eq!(actual, expected);
                }
            }
        }
        for content in [
            "ui = { sidebar = { agents = { tree = false, rows = [[\"agent\"]] } } }\n",
            "[ui.sidebar]\nagents = { tree = false, rows = [[\"agent\"]] }\n",
            "[ui.sidebar.agents]\ntree = false\ntree = true\n",
            "[ui.sidebar.agents]\nrows = [[\"agent\"]]\ntree\t= false\n",
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(update_file_at_checked(&path, "agent tree", |content| {
                ConfigEdit::AgentTree(true).checked_apply(content)
            })
            .is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn agent_tree_edit_rejects_multiline_rows_without_trailing_comma_without_writing() {
        let dir =
            std::env::temp_dir().join(format!("herdr-agent-tree-multiline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let content = "[ui.sidebar.agents]\nrows = [\n  [\"state_icon\", \"tab\"],\n  [\"agent\"]\n]\nrow_gap = 2\n";
        assert!(toml::from_str::<super::super::Config>(content).is_ok());
        std::fs::write(&path, content).unwrap();
        for enabled in [true, false] {
            let error = update_file_at_checked(&path, "agent tree", |content| {
                ConfigEdit::AgentTree(enabled).checked_apply(content)
            })
            .unwrap_err();
            assert!(
                error.contains(&format!(
                    "manually set tree = {enabled} under [ui.sidebar.agents]"
                )),
                "{error}"
            );
            assert!(!error.contains("instead of inline TOML"));
            assert_eq!(std::fs::read(&path).unwrap(), content.as_bytes());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_file_at_does_not_move_a_leading_bom_into_the_file() {
        let dir = std::env::temp_dir().join(format!("herdr-config-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            b"\xEF\xBB\xBF[terminal]\ndefault_shell = \"pwsh.exe\"\n",
        )
        .unwrap();

        update_file_at(&path, "onboarding setting", |content| {
            crate::config::upsert_top_level_bool(content, "onboarding", false)
        })
        .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            !written.contains('\u{feff}'),
            "unexpected BOM in {written:?}"
        );
        assert!(
            toml::from_str::<toml::Value>(&written).is_ok(),
            "written config is not valid TOML: {written:?}"
        );
    }
}
