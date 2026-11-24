use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug)]
pub enum Line {
    Section(String),
    KeyValue { key: String, value: String },
    Comment(String),
    Blank,
    Raw(String),
}

#[derive(Clone, Debug)]
pub struct DesktopDocument {
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalizedValue {
    pub key: String,
    pub locale: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLocalizedValue {
    pub key: String,
    pub locale: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopAction {
    pub original_id: Option<String>,
    pub id: String,
    pub name: String,
    pub icon: String,
    pub exec: String,
    pub localized_values: Vec<ActionLocalizedValue>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GroupKey {
    pub group: String,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtraKeyValue {
    pub group: String,
    pub key: String,
    pub value: String,
}

const LOCALIZED_KEYS: [&str; 5] = ["Name", "GenericName", "Comment", "Keywords", "Icon"];
const ACTION_GROUP_PREFIX: &str = "Desktop Action ";
const ACTION_LOCALIZED_KEYS: [&str; 2] = ["Name", "Icon"];
const MANAGED_MAIN_KEYS: [&str; 24] = [
    "Type",
    "Version",
    "Name",
    "GenericName",
    "NoDisplay",
    "Comment",
    "Icon",
    "Hidden",
    "OnlyShowIn",
    "NotShowIn",
    "DBusActivatable",
    "TryExec",
    "Exec",
    "Path",
    "Terminal",
    "Actions",
    "MimeType",
    "Categories",
    "Implements",
    "Keywords",
    "StartupNotify",
    "PrefersNonDefaultGPU",
    "SingleMainWindow",
    "URL",
];

impl DesktopDocument {
    pub fn parse(input: &str) -> Self {
        let mut lines = Vec::new();

        for source_line in input.lines() {
            let trimmed = source_line.trim();

            if trimmed.is_empty() {
                lines.push(Line::Blank);
                continue;
            }

            if trimmed.starts_with('#') {
                lines.push(Line::Comment(source_line.to_string()));
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() > 2 {
                lines.push(Line::Section(trimmed[1..trimmed.len() - 1].to_string()));
                continue;
            }

            if let Some((key, value)) = source_line.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    lines.push(Line::KeyValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                    continue;
                }
            }

            lines.push(Line::Raw(source_line.to_string()));
        }

        Self { lines }
    }

    pub fn new_application() -> Self {
        Self::parse(
            "[Desktop Entry]\nType=Application\nVersion=1.5\nName=New Application\nExec=\nIcon=application-x-executable\n",
        )
    }

    pub fn serialize(&self) -> String {
        let mut output = String::new();

        for line in &self.lines {
            match line {
                Line::Section(name) => {
                    output.push('[');
                    output.push_str(name);
                    output.push_str("]\n");
                }
                Line::KeyValue { key, value } => {
                    output.push_str(key);
                    output.push('=');
                    output.push_str(value);
                    output.push('\n');
                }
                Line::Comment(value) | Line::Raw(value) => {
                    output.push_str(value);
                    output.push('\n');
                }
                Line::Blank => output.push('\n'),
            }
        }

        output
    }

    pub fn get_main(&self, wanted: &str) -> Option<&str> {
        self.get_group_value("Desktop Entry", wanted)
    }

    pub fn localized_main(&self, wanted: &str) -> Option<&str> {
        let base = self.get_main(wanted)?;
        let locale = active_locale();

        for candidate in locale_candidates(&locale) {
            let key = format!("{wanted}[{candidate}]");
            if let Some(value) = self.get_main(&key) {
                return Some(value);
            }
        }

        Some(base)
    }

    pub fn display_name(&self) -> Option<String> {
        self.localized_main("Name").map(str::to_string)
    }

    pub fn set_main(&mut self, wanted: &str, value: Option<&str>) {
        self.ensure_main_group();
        self.set_group_value("Desktop Entry", wanted, value);
    }

    pub fn localized_values(&self) -> Vec<LocalizedValue> {
        let Some((start, end)) = self.group_range("Desktop Entry") else {
            return Vec::new();
        };
        let mut values = Vec::new();

        for line in &self.lines[start..end] {
            let Line::KeyValue { key, value } = line else {
                continue;
            };
            let Some((base, locale)) = split_localized_key(key) else {
                continue;
            };
            if is_localized_key(&base) {
                values.push(LocalizedValue {
                    key: base,
                    locale,
                    value: value.clone(),
                });
            }
        }

        values
    }

    pub fn replace_localized_values(&mut self, values: &[LocalizedValue]) -> Result<(), String> {
        validate_localized_values(values)?;
        self.ensure_main_group();

        if let Some((start, end)) = self.group_range("Desktop Entry") {
            let remove = (start..end)
                .filter(|index| {
                    let Line::KeyValue { key, .. } = &self.lines[*index] else {
                        return false;
                    };
                    split_localized_key(key).is_some_and(|(base, _)| is_localized_key(&base))
                })
                .collect::<Vec<_>>();

            for index in remove.into_iter().rev() {
                self.lines.remove(index);
            }
        }

        let mut insert_at = self
            .group_range("Desktop Entry")
            .map_or(self.lines.len(), |(_, end)| end);
        for value in values {
            self.lines.insert(
                insert_at,
                Line::KeyValue {
                    key: format!("{}[{}]", value.key.trim(), value.locale.trim()),
                    value: value.value.clone(),
                },
            );
            insert_at += 1;
        }

        Ok(())
    }

    pub fn desktop_actions(&self) -> Vec<DesktopAction> {
        self.action_ids()
            .into_iter()
            .map(|id| {
                let group = action_group_name(&id);
                DesktopAction {
                    original_id: Some(id.clone()),
                    id,
                    name: self
                        .get_group_value(&group, "Name")
                        .unwrap_or_default()
                        .to_string(),
                    icon: self
                        .get_group_value(&group, "Icon")
                        .unwrap_or_default()
                        .to_string(),
                    exec: self
                        .get_group_value(&group, "Exec")
                        .unwrap_or_default()
                        .to_string(),
                    localized_values: self.action_localized_values(&group),
                }
            })
            .collect()
    }

    pub fn key_value_map(&self) -> BTreeMap<GroupKey, Vec<String>> {
        let mut group = String::new();
        let mut values = BTreeMap::<GroupKey, Vec<String>>::new();

        for line in &self.lines {
            match line {
                Line::Section(name) => group.clone_from(name),
                Line::KeyValue { key, value } if !group.is_empty() => {
                    values
                        .entry(GroupKey {
                            group: group.clone(),
                            key: key.clone(),
                        })
                        .or_default()
                        .push(value.clone());
                }
                _ => {}
            }
        }

        values
    }

    pub fn replace_group_key_values(
        &mut self,
        group: &str,
        key: &str,
        values: &[String],
    ) -> Result<(), String> {
        validate_group_name(group)?;
        validate_key_name(key)?;
        if self.group_range(group).is_none() {
            if values.is_empty() {
                return Ok(());
            }
            self.append_group_block(vec![Line::Section(group.to_string())]);
        }

        let Some((start, end)) = self.group_range(group) else {
            return Err(format!("Could not create group [{group}]"));
        };
        let matches = (start..end)
            .filter(|index| {
                matches!(
                    &self.lines[*index],
                    Line::KeyValue { key: existing, .. } if existing == key
                )
            })
            .collect::<Vec<_>>();
        for index in matches.into_iter().rev() {
            self.lines.remove(index);
        }

        if values.is_empty() {
            return Ok(());
        }

        let mut insert_at = self
            .group_range(group)
            .map_or(self.lines.len(), |(_, group_end)| group_end);
        for value in values {
            self.lines.insert(
                insert_at,
                Line::KeyValue {
                    key: key.to_string(),
                    value: value.clone(),
                },
            );
            insert_at += 1;
        }
        Ok(())
    }

    pub fn extra_key_values(&self) -> Vec<ExtraKeyValue> {
        let mut group = String::new();
        let mut values = Vec::new();

        for line in &self.lines {
            match line {
                Line::Section(name) => group.clone_from(name),
                Line::KeyValue { key, value }
                    if !group.is_empty() && !is_managed_key(&group, key) =>
                {
                    values.push(ExtraKeyValue {
                        group: group.clone(),
                        key: key.clone(),
                        value: value.clone(),
                    });
                }
                _ => {}
            }
        }

        values
    }

    pub fn replace_extra_key_values(&mut self, values: &[ExtraKeyValue]) -> Result<(), String> {
        validate_extra_key_values(values)?;

        let mut group = String::new();
        self.lines = self
            .lines
            .iter()
            .filter_map(|line| {
                if let Line::Section(name) = line {
                    group.clone_from(name);
                    return Some(line.clone());
                }
                if let Line::KeyValue { key, .. } = line
                    && !group.is_empty()
                    && !is_managed_key(&group, key)
                {
                    return None;
                }
                Some(line.clone())
            })
            .collect();

        for value in values {
            self.replace_group_key_values(
                value.group.trim(),
                value.key.trim(),
                std::slice::from_ref(&value.value),
            )?;
        }
        Ok(())
    }

    pub fn replace_actions_and_extra(
        &mut self,
        actions: &[DesktopAction],
        values: &[ExtraKeyValue],
    ) -> Result<(), String> {
        let old_ids = self.action_ids();
        let mut remapped = Vec::with_capacity(values.len());
        for value in values {
            let mut value = value.clone();
            if let Some(old_id) = old_ids
                .iter()
                .find(|id| action_group_name(id) == value.group.trim())
            {
                let Some(action) = actions
                    .iter()
                    .find(|action| action.original_id.as_ref() == Some(old_id))
                else {
                    continue;
                };
                value.group = action_group_name(&action.id);
            }
            remapped.push(value);
        }
        let mut updated = self.clone();
        updated.replace_desktop_actions(actions)?;
        updated.replace_extra_key_values(&remapped)?;
        *self = updated;
        Ok(())
    }

    pub fn prune_type_specific_fields(&mut self) {
        match self.get_main("Type") {
            Some("Application") => self.set_main("URL", None),
            Some("Link" | "Directory") => {
                let is_directory = self.get_main("Type") == Some("Directory");
                for key in [
                    "Exec",
                    "TryExec",
                    "Path",
                    "Terminal",
                    "Actions",
                    "MimeType",
                    "Categories",
                    "Keywords",
                    "StartupNotify",
                    "StartupWMClass",
                    "DBusActivatable",
                    "PrefersNonDefaultGPU",
                    "SingleMainWindow",
                ] {
                    self.set_main(key, None);
                }
                if is_directory {
                    self.set_main("URL", None);
                }
                let mut group = String::new();
                self.lines.retain(|line| {
                    if let Line::Section(name) = line {
                        group.clone_from(name);
                    }
                    if group.starts_with(ACTION_GROUP_PREFIX) {
                        return false;
                    }
                    if group == "Desktop Entry"
                        && let Line::KeyValue { key, .. } = line
                    {
                        return split_localized_key(key).is_none_or(|(base, _)| base != "Keywords");
                    }
                    true
                });
            }
            _ => {}
        }
    }

    pub fn replace_desktop_actions(&mut self, actions: &[DesktopAction]) -> Result<(), String> {
        validate_actions(actions)?;

        let original_ids = self.action_ids();
        let managed_groups = original_ids
            .iter()
            .map(|id| action_group_name(id))
            .collect::<Vec<_>>();
        let unmanaged_groups = self
            .section_names()
            .into_iter()
            .filter(|name| !managed_groups.contains(name))
            .collect::<Vec<_>>();

        for action in actions {
            let target = action_group_name(&action.id);
            let owns_target = action
                .original_id
                .as_ref()
                .is_some_and(|original| action_group_name(original) == target);
            if unmanaged_groups.contains(&target) && !owns_target {
                return Err(format!("A group named [{target}] already exists"));
            }
        }

        let mut rebuilt = Vec::with_capacity(actions.len());
        for action in actions {
            let mut block = action
                .original_id
                .as_ref()
                .and_then(|id| self.group_block(&action_group_name(id)))
                .unwrap_or_else(|| vec![Line::Section(action_group_name(&action.id))]);
            update_action_block(&mut block, action);
            rebuilt.push(block);
        }

        for group in &managed_groups {
            self.remove_group(group);
        }

        for block in rebuilt {
            self.append_group_block(block);
        }

        let actions_value = (!actions.is_empty()).then(|| {
            format!(
                "{};",
                actions
                    .iter()
                    .map(|action| action.id.as_str())
                    .collect::<Vec<_>>()
                    .join(";")
            )
        });
        self.set_main("Actions", actions_value.as_deref());
        Ok(())
    }

    fn action_ids(&self) -> Vec<String> {
        self.get_main("Actions")
            .map(|value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn action_localized_values(&self, group: &str) -> Vec<ActionLocalizedValue> {
        let Some((start, end)) = self.group_range(group) else {
            return Vec::new();
        };

        self.lines[start..end]
            .iter()
            .filter_map(|line| {
                let Line::KeyValue { key, value } = line else {
                    return None;
                };
                let (base, locale) = split_localized_key(key)?;
                is_action_localized_key(&base).then(|| ActionLocalizedValue {
                    key: base,
                    locale,
                    value: value.clone(),
                })
            })
            .collect()
    }

    fn get_group_value(&self, group: &str, wanted: &str) -> Option<&str> {
        let (start, end) = self.group_range(group)?;

        self.lines[start..end].iter().find_map(|line| match line {
            Line::KeyValue { key, value } if key == wanted => Some(value.as_str()),
            _ => None,
        })
    }

    fn set_group_value(&mut self, group: &str, wanted: &str, value: Option<&str>) {
        let Some((start, end)) = self.group_range(group) else {
            return;
        };

        let matches = (start..end)
            .filter(|index| {
                matches!(
                    &self.lines[*index],
                    Line::KeyValue { key, .. } if key == wanted
                )
            })
            .collect::<Vec<_>>();

        if let Some(value) = value {
            if let Some(first) = matches.first().copied() {
                self.lines[first] = Line::KeyValue {
                    key: wanted.to_string(),
                    value: value.to_string(),
                };

                for index in matches.into_iter().skip(1).rev() {
                    self.lines.remove(index);
                }
            } else {
                let insert_at = self
                    .group_range(group)
                    .map_or(self.lines.len(), |(_, end)| end);
                self.lines.insert(
                    insert_at,
                    Line::KeyValue {
                        key: wanted.to_string(),
                        value: value.to_string(),
                    },
                );
            }
        } else {
            for index in matches.into_iter().rev() {
                self.lines.remove(index);
            }
        }
    }

    fn group_range(&self, wanted: &str) -> Option<(usize, usize)> {
        let mut start = None;

        for (index, line) in self.lines.iter().enumerate() {
            match line {
                Line::Section(name) if name == wanted => start = Some(index + 1),
                Line::Section(_) if start.is_some() => return start.map(|value| (value, index)),
                _ => {}
            }
        }

        start.map(|value| (value, self.lines.len()))
    }

    fn group_block(&self, group: &str) -> Option<Vec<Line>> {
        let (start, end) = self.group_range(group)?;
        let section_index = start.checked_sub(1)?;
        Some(self.lines[section_index..end].to_vec())
    }

    fn remove_group(&mut self, group: &str) {
        let Some((start, end)) = self.group_range(group) else {
            return;
        };
        let Some(section_index) = start.checked_sub(1) else {
            return;
        };
        self.lines.drain(section_index..end);
    }

    fn append_group_block(&mut self, block: Vec<Line>) {
        if !self.lines.is_empty() && !matches!(self.lines.last(), Some(Line::Blank)) {
            self.lines.push(Line::Blank);
        }
        self.lines.extend(block);
    }

    fn section_names(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Section(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    fn ensure_main_group(&mut self) {
        if self.group_range("Desktop Entry").is_none() {
            self.lines
                .insert(0, Line::Section("Desktop Entry".to_string()));
        }
    }
}

fn is_managed_key(group: &str, key: &str) -> bool {
    if group == "Desktop Entry" {
        if MANAGED_MAIN_KEYS.contains(&key) {
            return true;
        }
        return split_localized_key(key).is_some_and(|(base, _)| is_localized_key(&base));
    }

    if group.starts_with(ACTION_GROUP_PREFIX) {
        if matches!(key, "Name" | "Icon" | "Exec") {
            return true;
        }
        return split_localized_key(key).is_some_and(|(base, _)| is_action_localized_key(&base));
    }

    false
}

fn validate_extra_key_values(values: &[ExtraKeyValue]) -> Result<(), String> {
    let mut seen = HashSet::with_capacity(values.len());
    for value in values {
        let group = value.group.trim();
        let key = value.key.trim();
        validate_group_name(group)?;
        validate_key_name(key)?;
        if is_managed_key(group, key) {
            return Err(format!(
                "Key '{key}' in [{group}] is managed by a dedicated DeskEntry field"
            ));
        }
        if !seen.insert((group, key)) {
            return Err(format!("Extra key '{key}' in [{group}] is duplicated"));
        }
    }
    Ok(())
}

fn validate_group_name(group: &str) -> Result<(), String> {
    if group.is_empty() {
        return Err("Group name must not be empty".to_string());
    }
    if group
        .chars()
        .any(|character| character.is_control() || matches!(character, '[' | ']'))
    {
        return Err(format!(
            "Group name '{group}' contains unsupported characters"
        ));
    }
    Ok(())
}

fn validate_key_name(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Key name must not be empty".to_string());
    }
    if let Some((base, locale)) = split_localized_key(key) {
        validate_plain_key_name(&base)?;
        return validate_locale_identifier(&locale);
    }
    validate_plain_key_name(key)
}

fn validate_plain_key_name(key: &str) -> Result<(), String> {
    if key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err(format!(
            "Key name '{key}' may contain only A-Z, a-z, 0-9 and '-'"
        ))
    }
}

fn action_group_name(id: &str) -> String {
    format!("{ACTION_GROUP_PREFIX}{id}")
}

fn validate_localized_values(values: &[LocalizedValue]) -> Result<(), String> {
    let mut keys = HashSet::with_capacity(values.len());

    for value in values {
        let key = value.key.trim();
        let locale = value.locale.trim();
        if !is_localized_key(key) {
            return Err(format!("Unsupported localized key '{key}'"));
        }
        validate_locale_identifier(locale)?;
        if !keys.insert((key, locale)) {
            return Err(format!("Localized key '{key}[{locale}]' is duplicated"));
        }
    }

    Ok(())
}

fn validate_locale_identifier(locale: &str) -> Result<(), String> {
    if locale.is_empty() {
        return Err("Locale identifier must not be empty".to_string());
    }
    if !locale.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '@' | '-')
    }) {
        return Err(format!(
            "Locale identifier '{locale}' contains unsupported characters"
        ));
    }
    Ok(())
}

fn validate_actions(actions: &[DesktopAction]) -> Result<(), String> {
    let mut ids = HashSet::with_capacity(actions.len());

    for action in actions {
        let id = action.id.trim();
        if id.is_empty() {
            return Err("Desktop Action ID must not be empty".to_string());
        }
        if id != action.id {
            return Err(format!(
                "Desktop Action ID '{}' must not contain leading or trailing whitespace",
                action.id
            ));
        }
        if !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(format!(
                "Desktop Action ID '{id}' may contain only A-Z, a-z, 0-9 and '-'"
            ));
        }
        if action.name.trim().is_empty() {
            return Err(format!("Desktop Action '{id}' requires a Name"));
        }
        if !ids.insert(id) {
            return Err(format!("Desktop Action ID '{id}' is duplicated"));
        }
        let mut localized_keys = HashSet::with_capacity(action.localized_values.len());
        for value in &action.localized_values {
            let key = value.key.trim();
            let locale = value.locale.trim();
            if !is_action_localized_key(key) {
                return Err(format!(
                    "Desktop Action '{id}' contains unsupported localized key '{}'",
                    value.key
                ));
            }
            validate_locale_identifier(locale)
                .map_err(|error| format!("Desktop Action '{id}' has an invalid locale: {error}"))?;
            if !localized_keys.insert((key, locale)) {
                return Err(format!(
                    "Desktop Action '{id}' has duplicated localized key '{key}[{locale}]'"
                ));
            }
        }
    }

    Ok(())
}

fn update_action_block(block: &mut Vec<Line>, action: &DesktopAction) {
    if let Some(Line::Section(name)) = block.first_mut() {
        *name = action_group_name(&action.id);
    } else {
        block.insert(0, Line::Section(action_group_name(&action.id)));
    }

    set_block_value(block, "Name", Some(action.name.as_str()));
    let has_localized_icon = action
        .localized_values
        .iter()
        .any(|value| value.key == "Icon");
    set_block_value(
        block,
        "Icon",
        (!action.icon.trim().is_empty() || has_localized_icon).then_some(action.icon.as_str()),
    );
    set_block_value(
        block,
        "Exec",
        (!action.exec.trim().is_empty()).then_some(action.exec.as_str()),
    );
    replace_block_localized_values(block, &action.localized_values);
}

fn set_block_value(block: &mut Vec<Line>, wanted: &str, value: Option<&str>) {
    let matches = block
        .iter()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Line::KeyValue { key, .. } if key == wanted => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();

    if let Some(value) = value {
        if let Some(first) = matches.first().copied() {
            block[first] = Line::KeyValue {
                key: wanted.to_string(),
                value: value.to_string(),
            };
            for index in matches.into_iter().skip(1).rev() {
                block.remove(index);
            }
        } else {
            block.push(Line::KeyValue {
                key: wanted.to_string(),
                value: value.to_string(),
            });
        }
    } else {
        for index in matches.into_iter().rev() {
            block.remove(index);
        }
    }
}

fn replace_block_localized_values(block: &mut Vec<Line>, values: &[ActionLocalizedValue]) {
    let remove = block
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let Line::KeyValue { key, .. } = line else {
                return None;
            };
            split_localized_key(key)
                .is_some_and(|(base, _)| is_action_localized_key(&base))
                .then_some(index)
        })
        .collect::<Vec<_>>();

    for index in remove.into_iter().rev() {
        block.remove(index);
    }

    for value in values {
        block.push(Line::KeyValue {
            key: format!("{}[{}]", value.key.trim(), value.locale.trim()),
            value: value.value.clone(),
        });
    }
}

fn is_localized_key(key: &str) -> bool {
    LOCALIZED_KEYS.contains(&key)
}

fn is_action_localized_key(key: &str) -> bool {
    ACTION_LOCALIZED_KEYS.contains(&key)
}

fn split_localized_key(key: &str) -> Option<(String, String)> {
    let open = key.find('[')?;
    if !key.ends_with(']') || open == 0 {
        return None;
    }

    Some((
        key[..open].to_string(),
        key[open + 1..key.len() - 1].to_string(),
    ))
}

fn active_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let locale = locale.trim();
    if locale.is_empty() {
        return Vec::new();
    }

    let (before_modifier, modifier) = locale
        .split_once('@')
        .map_or((locale, None), |(base, modifier)| (base, Some(modifier)));
    let base = before_modifier
        .split_once('.')
        .map_or(before_modifier, |(value, _)| value);

    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }

    let language = base.split_once('_').map_or(base, |(value, _)| value);
    let mut candidates = Vec::with_capacity(4);

    if let Some(modifier) = modifier.filter(|value| !value.is_empty()) {
        push_unique(&mut candidates, format!("{base}@{modifier}"));
    }
    push_unique(&mut candidates, base.to_string());
    if let Some(modifier) = modifier.filter(|value| !value.is_empty()) {
        push_unique(&mut candidates, format!("{language}@{modifier}"));
    }
    push_unique(&mut candidates, language.to_string());

    candidates
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_groups_and_comments() {
        let input = "# vendor\n[Desktop Entry]\nType=Application\nName=Test\nX-Vendor-Key=yes\n\n[Desktop Action Foo]\nName=Foo\nExec=test --foo\n";
        let mut document = DesktopDocument::parse(input);
        document.set_main("Name", Some("Changed"));
        let output = document.serialize();
        assert!(output.contains("# vendor"));
        assert!(output.contains("X-Vendor-Key=yes"));
        assert!(output.contains("[Desktop Action Foo]"));
        assert!(output.contains("Exec=test --foo"));
        assert!(output.contains("Name=Changed"));
    }

    #[test]
    fn replaces_localized_values() -> Result<(), String> {
        let mut document = DesktopDocument::parse(
            "[Desktop Entry]\nType=Application\nName=Test\nName[pl]=Test PL\nComment[de]=Alt\n",
        );
        document.replace_localized_values(&[LocalizedValue {
            key: "Name".to_string(),
            locale: "pl".to_string(),
            value: "Nowa nazwa".to_string(),
        }])?;
        let output = document.serialize();
        assert!(output.contains("Name[pl]=Nowa nazwa"));
        assert!(!output.contains("Comment[de]"));
        Ok(())
    }

    #[test]
    fn locale_fallback_order_matches_specification() {
        assert_eq!(
            locale_candidates("sr_RS.UTF-8@latin"),
            vec!["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert!(locale_candidates("C.UTF-8").is_empty());
    }

    #[test]
    fn get_main_borrows_value() {
        let document = DesktopDocument::parse("[Desktop Entry]\nName=Test\n");
        assert_eq!(document.get_main("Name"), Some("Test"));
    }

    #[test]
    fn preserves_semicolon_prefixed_raw_lines() {
        let input = "[Desktop Entry]\nName=Test\n;vendor-specific-line\n";
        let document = DesktopDocument::parse(input);
        assert_eq!(document.serialize(), input);
    }

    #[test]
    fn rejects_duplicate_localized_keys() {
        let mut document = DesktopDocument::new_application();
        let values = [
            LocalizedValue {
                key: "Name".to_string(),
                locale: "pl".to_string(),
                value: "Pierwsza".to_string(),
            },
            LocalizedValue {
                key: "Name".to_string(),
                locale: "pl".to_string(),
                value: "Druga".to_string(),
            },
        ];
        assert!(document.replace_localized_values(&values).is_err());
    }

    #[test]
    fn rejects_invalid_locale_identifiers() {
        let mut document = DesktopDocument::new_application();
        let values = [LocalizedValue {
            key: "Name".to_string(),
            locale: "pl]bad".to_string(),
            value: "Bad".to_string(),
        }];
        assert!(document.replace_localized_values(&values).is_err());
    }

    #[test]
    fn edits_actions_without_dropping_unknown_keys() -> Result<(), String> {
        let input = "[Desktop Entry]\nType=Application\nName=Test\nActions=Open;\n\n[Desktop Action Open]\n# keep\nName=Open\nName[pl]=Otwórz\nExec=test --open\nX-Vendor-Key=yes\n\n[Vendor Group]\nValue=keep\n";
        let mut document = DesktopDocument::parse(input);
        document.replace_desktop_actions(&[DesktopAction {
            original_id: Some("Open".to_string()),
            id: "Launch".to_string(),
            name: "Launch".to_string(),
            icon: "system-run".to_string(),
            exec: "test --launch".to_string(),
            localized_values: vec![
                ActionLocalizedValue {
                    key: "Name".to_string(),
                    locale: "pl".to_string(),
                    value: "Uruchom".to_string(),
                },
                ActionLocalizedValue {
                    key: "Icon".to_string(),
                    locale: "pl".to_string(),
                    value: "system-run".to_string(),
                },
            ],
        }])?;

        let output = document.serialize();
        assert!(output.contains("Actions=Launch;"));
        assert!(output.contains("[Desktop Action Launch]"));
        assert!(output.contains("# keep"));
        assert!(output.contains("X-Vendor-Key=yes"));
        assert!(output.contains("Name[pl]=Uruchom"));
        assert!(output.contains("Icon[pl]=system-run"));
        assert!(output.contains("[Vendor Group]"));
        assert!(output.contains("Value=keep"));
        assert!(!output.contains("[Desktop Action Open]"));
        Ok(())
    }

    #[test]
    fn action_rename_and_delete_remap_extra_keys() -> Result<(), String> {
        let input = "[Desktop Entry]\nType=Application\nName=Test\nActions=Open;Remove;\n[Desktop Action Open]\nName=Open\nExec=test\nX-Vendor=keep\n[Desktop Action Remove]\nName=Remove\nExec=test\nX-Vendor=drop\n[Vendor Group]\nKey=keep\n";
        let mut document = DesktopDocument::parse(input);
        let mut actions = document.desktop_actions();
        actions.retain(|action| action.id == "Open");
        if let Some(action) = actions.first_mut() {
            action.id = "Launch".to_string();
        }
        let mut values = document.extra_key_values();
        for value in &mut values {
            if value.group == "Desktop Action Open" {
                value.value = "edited".to_string();
            }
        }
        document.replace_actions_and_extra(&actions, &values)?;
        assert_eq!(document.get_main("Actions"), Some("Launch;"));
        assert_eq!(
            document.get_group_value("Desktop Action Launch", "X-Vendor"),
            Some("edited")
        );
        assert_eq!(
            document.get_group_value("Vendor Group", "Key"),
            Some("keep")
        );
        assert!(!document.serialize().contains("[Desktop Action Open]"));
        assert!(!document.serialize().contains("[Desktop Action Remove]"));
        Ok(())
    }

    #[test]
    fn swapped_actions_keep_their_own_extra_keys() -> Result<(), String> {
        let input = "[Desktop Entry]\nType=Application\nName=Test\nActions=A;B;\n[Desktop Action A]\nName=First\nExec=test\nX-Key=one\n[Desktop Action B]\nName=Second\nExec=test\nX-Key=two\n";
        let mut document = DesktopDocument::parse(input);
        let mut actions = document.desktop_actions();
        for action in &mut actions {
            action.id = if action.id == "A" { "B" } else { "A" }.to_string();
        }
        let values = document.extra_key_values();
        document.replace_actions_and_extra(&actions, &values)?;
        assert_eq!(
            document.get_group_value("Desktop Action B", "X-Key"),
            Some("one")
        );
        assert_eq!(
            document.get_group_value("Desktop Action A", "X-Key"),
            Some("two")
        );
        Ok(())
    }

    #[test]
    fn type_changes_remove_incompatible_fields() {
        let input = "[Desktop Entry]\nType=Link\nName=Test\nExec=test\nURL=https://example.org\nKeywords=test;\nKeywords[pl]=test;\nStartupWMClass=test\nTerminal=false\nActions=Open;\nX-Vendor=keep\n[Desktop Action Open]\nName=Open\nExec=test\nX-Key=drop\n[Vendor Group]\nKey=keep\n";
        let mut document = DesktopDocument::parse(input);
        document.prune_type_specific_fields();
        for key in [
            "Exec",
            "Keywords",
            "Keywords[pl]",
            "StartupWMClass",
            "Terminal",
            "Actions",
        ] {
            assert_eq!(document.get_main(key), None);
        }
        assert_eq!(document.get_main("URL"), Some("https://example.org"));
        assert_eq!(document.get_main("X-Vendor"), Some("keep"));
        assert_eq!(
            document.get_group_value("Vendor Group", "Key"),
            Some("keep")
        );
        assert!(!document.serialize().contains("[Desktop Action Open]"));
        document.set_main("Type", Some("Application"));
        document.prune_type_specific_fields();
        assert_eq!(document.get_main("URL"), None);
        document.set_main("Type", Some("Directory"));
        document.set_main("URL", Some("https://example.org"));
        document.prune_type_specific_fields();
        assert_eq!(document.get_main("URL"), None);
    }

    #[test]
    fn preserves_orphan_action_groups() -> Result<(), String> {
        let input = "[Desktop Entry]\nType=Application\nName=Test\nActions=Open;\n\n[Desktop Action Open]\nName=Open\nExec=test --open\n\n[Desktop Action Orphan]\nName=Orphan\nX-Vendor=yes\n";
        let mut document = DesktopDocument::parse(input);
        document.replace_desktop_actions(&[])?;
        let output = document.serialize();
        assert!(!output.contains("[Desktop Action Open]"));
        assert!(output.contains("[Desktop Action Orphan]"));
        assert!(output.contains("X-Vendor=yes"));
        Ok(())
    }

    #[test]
    fn preserves_empty_base_icon_for_localized_action_icon() -> Result<(), String> {
        let mut document = DesktopDocument::new_application();
        document.replace_desktop_actions(&[DesktopAction {
            original_id: None,
            id: "Open".to_string(),
            name: "Open".to_string(),
            icon: String::new(),
            exec: "test --open".to_string(),
            localized_values: vec![ActionLocalizedValue {
                key: "Icon".to_string(),
                locale: "pl".to_string(),
                value: "system-run".to_string(),
            }],
        }])?;

        let output = document.serialize();
        assert!(output.contains("Icon=\n"));
        assert!(output.contains("Icon[pl]=system-run"));
        Ok(())
    }

    #[test]
    fn rejects_duplicate_action_locales() {
        let mut document = DesktopDocument::new_application();
        let result = document.replace_desktop_actions(&[DesktopAction {
            original_id: None,
            id: "Open".to_string(),
            name: "Open".to_string(),
            icon: String::new(),
            exec: "test --open".to_string(),
            localized_values: vec![
                ActionLocalizedValue {
                    key: "Name".to_string(),
                    locale: "pl".to_string(),
                    value: "Otwórz".to_string(),
                },
                ActionLocalizedValue {
                    key: "Name".to_string(),
                    locale: "pl".to_string(),
                    value: "Uruchom".to_string(),
                },
            ],
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_action_ids() {
        let mut document = DesktopDocument::new_application();
        let result = document.replace_desktop_actions(&[DesktopAction {
            original_id: None,
            id: "Bad ID".to_string(),
            name: "Bad".to_string(),
            icon: String::new(),
            exec: String::new(),
            localized_values: Vec::new(),
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn exposes_and_replaces_extra_keys_without_dropping_comments() -> Result<(), String> {
        let input = "# keep\n[Desktop Entry]\nType=Application\nName=Test\nX-Vendor=yes\n\n[Vendor Group]\nValue=old\n";
        let mut document = DesktopDocument::parse(input);
        assert_eq!(
            document.extra_key_values(),
            vec![
                ExtraKeyValue {
                    group: "Desktop Entry".to_string(),
                    key: "X-Vendor".to_string(),
                    value: "yes".to_string(),
                },
                ExtraKeyValue {
                    group: "Vendor Group".to_string(),
                    key: "Value".to_string(),
                    value: "old".to_string(),
                },
            ]
        );
        document.replace_extra_key_values(&[ExtraKeyValue {
            group: "Desktop Entry".to_string(),
            key: "X-Vendor".to_string(),
            value: "changed".to_string(),
        }])?;
        let output = document.serialize();
        assert!(output.contains("# keep"));
        assert!(output.contains("X-Vendor=changed"));
        assert!(!output.contains("Value=old"));
        Ok(())
    }

    #[test]
    fn key_value_map_preserves_duplicate_values_for_comparison() {
        let document = DesktopDocument::parse(
            "[Desktop Entry]\nName=First\nName=Second\n\n[Vendor]\nKey=Value\n",
        );
        let values = document.key_value_map();
        assert_eq!(
            values.get(&GroupKey {
                group: "Desktop Entry".to_string(),
                key: "Name".to_string(),
            }),
            Some(&vec!["First".to_string(), "Second".to_string()])
        );
    }

    #[test]
    fn replaces_group_key_values_and_can_create_group() -> Result<(), String> {
        let mut document = DesktopDocument::new_application();
        document.replace_group_key_values(
            "Vendor Group",
            "X-Value",
            &["one".to_string(), "two".to_string()],
        )?;
        let output = document.serialize();
        assert!(output.contains("[Vendor Group]"));
        assert!(output.contains("X-Value=one"));
        assert!(output.contains("X-Value=two"));
        Ok(())
    }
}
