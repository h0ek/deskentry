use crate::model::{DesktopDocument, GroupKey};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyDifference {
    pub key: GroupKey,
    pub override_values: Vec<String>,
    pub original_values: Vec<String>,
}

pub fn compare_documents(
    override_document: &DesktopDocument,
    original_document: &DesktopDocument,
) -> Vec<KeyDifference> {
    let override_values = override_document.key_value_map();
    let original_values = original_document.key_value_map();
    let keys = override_values
        .keys()
        .chain(original_values.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    keys.into_iter()
        .filter_map(|key| {
            let current = override_values.get(&key).cloned().unwrap_or_default();
            let original = original_values.get(&key).cloned().unwrap_or_default();
            (current != original).then_some(KeyDifference {
                key,
                override_values: current,
                original_values: original,
            })
        })
        .collect()
}

pub fn apply_original_value(
    document: &mut DesktopDocument,
    difference: &KeyDifference,
) -> Result<(), String> {
    document.replace_group_key_values(
        &difference.key.group,
        &difference.key.key,
        &difference.original_values,
    )
}

pub fn display_values(values: &[String]) -> String {
    if values.is_empty() {
        return "Not present".to_string();
    }
    if values.len() == 1 {
        return values[0].clone();
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("{}: {value}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semantic_keys_instead_of_line_order() {
        let left =
            DesktopDocument::parse("[Desktop Entry]\nType=Application\nName=Test\nIcon=test\n");
        let right =
            DesktopDocument::parse("[Desktop Entry]\nIcon=test\nName=Test\nType=Application\n");
        assert!(compare_documents(&left, &right).is_empty());
    }

    #[test]
    fn detects_added_removed_and_changed_values() {
        let override_document = DesktopDocument::parse(
            "[Desktop Entry]\nType=Application\nName=Custom\nX-Only-Override=yes\n",
        );
        let original_document = DesktopDocument::parse(
            "[Desktop Entry]\nType=Application\nName=Upstream\nIcon=upstream\n",
        );
        let differences = compare_documents(&override_document, &original_document);
        assert_eq!(differences.len(), 3);
        assert!(differences.iter().any(|difference| {
            difference.key.key == "Name"
                && difference.override_values == ["Custom"]
                && difference.original_values == ["Upstream"]
        }));
        assert!(differences.iter().any(|difference| {
            difference.key.key == "Icon"
                && difference.override_values.is_empty()
                && difference.original_values == ["upstream"]
        }));
        assert!(differences.iter().any(|difference| {
            difference.key.key == "X-Only-Override"
                && difference.override_values == ["yes"]
                && difference.original_values.is_empty()
        }));
    }

    #[test]
    fn applies_selected_original_value() -> Result<(), String> {
        let mut override_document = DesktopDocument::parse(
            "[Desktop Entry]\nType=Application\nName=Custom\nX-Vendor=yes\n",
        );
        let original_document =
            DesktopDocument::parse("[Desktop Entry]\nType=Application\nName=Upstream\n");
        let difference = compare_documents(&override_document, &original_document)
            .into_iter()
            .find(|difference| difference.key.key == "Name")
            .ok_or_else(|| "Name difference not found".to_string())?;
        apply_original_value(&mut override_document, &difference)?;
        assert_eq!(override_document.get_main("Name"), Some("Upstream"));
        assert_eq!(override_document.get_main("X-Vendor"), Some("yes"));
        Ok(())
    }

    #[test]
    fn applying_original_absence_removes_override_only_key() -> Result<(), String> {
        let mut override_document = DesktopDocument::parse(
            "[Desktop Entry]\nType=Application\nName=Test\nX-DeskEntry-Test=yes\n",
        );
        let original_document =
            DesktopDocument::parse("[Desktop Entry]\nType=Application\nName=Test\n");
        let difference = compare_documents(&override_document, &original_document)
            .into_iter()
            .find(|difference| difference.key.key == "X-DeskEntry-Test")
            .ok_or_else(|| "Override-only difference not found".to_string())?;
        apply_original_value(&mut override_document, &difference)?;
        assert_eq!(override_document.get_main("X-DeskEntry-Test"), None);
        Ok(())
    }
}
