use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use crate::features::agenda::AgendaCommand;
pub use crate::features::assistant::AssistantCommand;
pub use crate::features::local::LocalCommand;
pub use crate::features::note::NoteCommand;
pub use crate::features::notes::NotesCommand;
pub use crate::features::permissions::PermissionsCommand;
pub use crate::features::recording::RecordingCommand;
pub use crate::features::status::StatusCommand;

app_type! {
    #[serde(untagged)]
    pub enum AppCommand {
        Status(StatusCommand),
        Permissions(PermissionsCommand),
        Agenda(AgendaCommand),
        Notes(NotesCommand),
        Note(NoteCommand),
        Recording(RecordingCommand),
        Assistant(AssistantCommand),
        Local(LocalCommand),
        Daemon(Box<orbit_core::protocol::Command>),
    }
}

app_type! {
    /// Orders accepted by the floating call panel. Window-local orders stay
    /// in Tauri; the six recording orders use the core command unchanged.
    #[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
    pub enum PanelOrder {
        PanelFocus { focus: bool },
        OpenUrl { url: String },
        CallRecord { call_id: String },
        CallDismiss { call_id: String },
        RecordingKeep { recording_id: String },
        MeetingFinish,
        MeetingAsk { question: String, minutes: u16 },
        MeetingQuestionClose { id: String },
    }
}

#[cfg(test)]
mod wire_tests {
    fn feature(command: &super::AppCommand) -> crate::features::FeatureId {
        use super::AppCommand;
        use crate::features::FeatureId;
        match command {
            AppCommand::Daemon(command) => command
                .feature()
                .unwrap_or_else(|| panic!("internal fixture {command:?}")),
            AppCommand::Status(_) => FeatureId::Status,
            AppCommand::Permissions(_) => FeatureId::Permissions,
            AppCommand::Agenda(_) => FeatureId::Agenda,
            AppCommand::Notes(_) => FeatureId::Notes,
            AppCommand::Note(_) => FeatureId::Note,
            AppCommand::Recording(_) => FeatureId::Recording,
            AppCommand::Assistant(_) => FeatureId::Assistant,
            AppCommand::Local(_) => FeatureId::Local,
        }
    }
    use super::*;
    use crate::features::FeatureId;
    use serde_json::json;
    mod interface_orders {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/interface_orders.rs"
        ));
    }

    #[test]
    fn every_app_command_round_trips_in_its_feature() -> Result<(), serde_json::Error> {
        let cases = interface_orders::cases();
        #[derive(Default)]
        struct Vocabulary {
            seen: std::collections::HashSet<std::any::TypeId>,
            declarations: Vec<String>,
        }
        impl ts_rs::TypeVisitor for Vocabulary {
            fn visit<T: TS + 'static + ?Sized>(&mut self) {
                if !self.seen.insert(std::any::TypeId::of::<T>()) {
                    return;
                }
                if T::output_path().is_some() {
                    self.declarations.push(T::decl());
                }
                T::visit_dependencies(self);
            }
        }
        let mut vocabulary = Vocabulary::default();
        AppCommand::visit_dependencies(&mut vocabulary);
        let exported = vocabulary.declarations.join(" ");
        let names: std::collections::BTreeSet<_> = exported
            .split("\"command\": \"")
            .skip(1)
            .filter_map(|part| part.split('"').next())
            .collect();
        let covered: std::collections::BTreeSet<_> = cases
            .iter()
            .filter_map(|(value, _)| value["command"].as_str())
            .collect();
        assert_eq!(
            names, covered,
            "exported orders missing from the wire cases"
        );
        for (command, declaration) in [
            (
                "calendar",
                orbit_core::calendar_control::CalendarCommand::decl(),
            ),
            ("note_library", orbit_core::notes::LibraryRequest::decl()),
        ] {
            let actions: std::collections::BTreeSet<_> = declaration
                .split("\"action\": \"")
                .skip(1)
                .filter_map(|part| part.split('"').next())
                .collect();
            let covered: std::collections::BTreeSet<_> = cases
                .iter()
                .filter(|(value, _)| value["command"] == command)
                .filter_map(|(value, _)| value["request"]["action"].as_str())
                .collect();
            assert_eq!(actions, covered, "missing nested order for {command}");
        }
        let mut features = std::collections::BTreeMap::new();
        for (value, expected_feature) in cases {
            let command: AppCommand = serde_json::from_value(value.clone())?;
            let encoded = serde_json::to_value(&command)?;
            assert_eq!(encoded, value, "wire shape changed for {command:?}");
            let again: AppCommand = serde_json::from_value(encoded)?;
            assert_eq!(again, command);
            features.insert(
                value["command"].as_str().unwrap_or_default().to_owned(),
                feature(&again),
            );
            assert_eq!(
                feature(&again),
                expected_feature,
                "wrong feature for {again:?}"
            );
        }
        let mut generated = String::from(
            "// Generated by orbit-app-core's command vocabulary test. Do not edit.\nimport type { AppCommand } from \"./AppCommand\";\nimport type { FeatureId } from \"./FeatureId\";\nexport const commandFeature = ",
        );
        generated.push_str(&serde_json::to_string_pretty(&features)?);
        generated.push_str(" as const satisfies Record<AppCommand[\"command\"], FeatureId>;\n");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src/bridge/generated/commandFeatures.ts");
        std::fs::write(path, generated).map_err(serde_json::Error::io)?;
        let declaration = PanelOrder::decl();
        let orders: std::collections::BTreeMap<_, _> = declaration
            .split("\"command\": \"")
            .skip(1)
            .filter_map(|part| part.split('"').next())
            .map(|name| (name, true))
            .collect();
        let generated = format!(
            "// Generated from PanelOrder by orbit-app-core's command vocabulary test. Do not edit.\nimport type {{ PanelOrder }} from \"./PanelOrder\";\nexport const panelOrders = {} as const satisfies Record<PanelOrder[\"command\"], true>;\n",
            serde_json::to_string_pretty(&orders)?
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src/bridge/generated/panelOrders.ts");
        std::fs::write(path, generated).map_err(serde_json::Error::io)?;
        Ok(())
    }

    #[test]
    fn event_start_decodes_with_only_the_selected_observation() -> Result<(), serde_json::Error> {
        let command: AppCommand = serde_json::from_value(json!({
            "command":"meeting_start", "event_id":"event-1"
        }))?;
        assert!(
            matches!(&command, AppCommand::Recording(RecordingCommand::MeetingStartSelected {
            event_id: Some(id), ..
        }) if id == "event-1")
        );
        assert_eq!(feature(&command), FeatureId::Recording);
        let encoded = serde_json::to_value(&command)?;
        assert_eq!(serde_json::from_value::<AppCommand>(encoded)?, command);
        Ok(())
    }
}
