//! The hub link's events: `ready` with the hub's protocol, its storage and
//! its agents, and the agents it lists again later.
use super::*;

impl Runtime {
    pub(super) async fn on_hub_ready(&mut self, data: &ReadyData) -> Result<()> {
        self.link.protocol_version = data.hub_protocol;
        if !self.settings.device_id.as_deref().is_some_and(|expected| {
            !expected.is_empty() && data.device_id.as_deref() == Some(expected)
        }) {
            self.apply(Event::Connect(Connection::Revoked)).await?;
            if data.device_id.is_none() {
                self.record_invalid_hub_frame(
                    "ready",
                    "missing field `device_id`: expected string",
                );
                self.say(NoticeCode::HubDeviceIdentityMissing);
            } else if self
                .settings
                .device_id
                .as_deref()
                .is_some_and(|expected| !expected.is_empty())
            {
                self.say(
                    Notice::new(NoticeCode::DevicePairingMismatch)
                        .with_token(NoticeKey::Cause, "other_device"),
                );
            } else {
                self.say(
                    Notice::new(NoticeCode::DevicePairingMismatch)
                        .with_token(NoticeKey::Cause, "no_local_identity"),
                );
            }
        } else {
            let mut settings = self.settings.clone();
            if let Some(name) = &data.assistant_name {
                settings.assistant_name = Some(name.clone());
            }
            settings.hub_agents = self.learned_agents(data);
            // What this hub takes now: a hub that names no ceilings has no
            // attachments (0.9.0), and the composer offers none.
            self.assistant.hub_attachments = data.attachments;
            if settings.assistant_name != self.settings.assistant_name
                || settings.hub_agents != self.settings.hub_agents
            {
                self.paths.save_settings(&settings)?;
                self.settings = settings;
                if let Some(stream) = self.question.wake_stream.clone() {
                    self.send_wake_order(stream, true).await;
                }
            }
            self.notes.deletions.hub_deletes = Some(data.recording_delete == Some(true));
            self.notes.notes_hub_ready();
            self.apply(Event::Connect(Connection::Online)).await?;
            if let Some(storage) = &data.storage {
                self.on_hub_storage(storage);
            }
        }
        Ok(())
    }

    /// What the hub's storage is: said once when the hub becomes nearly full
    /// (`orbit_core::hub_storage`), with its numbers.
    pub(super) fn on_hub_storage(&mut self, storage: &orbit_core::wire::hub::HubStorage) {
        if let Some(notice) =
            orbit_core::hub_storage::storage_notice(self.link.storage_nearly_full, storage)
        {
            self.say(notice);
        }
        self.link.storage_nearly_full = Some(storage.nearly_full);
    }

    /// The hub's agents changed (a profile, one added or removed, an
    /// account): the Mac keeps the new list, and the wake phrase hears its
    /// names, as after `ready`.
    pub(super) async fn on_hub_agents_changed(&mut self, data: &AgentsChangedData) -> Result<()> {
        let Some(roster) = self.admitted_agents(
            &data.agents,
            data.default_agent_id.clone(),
            data.addable_agents.clone(),
        ) else {
            return Ok(());
        };
        if self.settings.hub_agents.as_ref() == Some(&roster) {
            return Ok(());
        }
        let mut settings = self.settings.clone();
        settings.assistant_name = Some(roster.default_name().to_owned());
        settings.hub_agents = Some(roster);
        self.paths.save_settings(&settings)?;
        self.settings = settings;
        if let Some(stream) = self.question.wake_stream.clone() {
            self.send_wake_order(stream, true).await;
        }
        Ok(())
    }

    /// The agents this hub has, from its `ready`: none listed is a hub
    /// with one (0.9.0); a list the Mac cannot rely on is logged with its
    /// cause and numbers, and the one the Mac had stays.
    pub(super) fn learned_agents(
        &self,
        data: &ReadyData,
    ) -> Option<orbit_core::agents::AgentRoster> {
        self.admitted_agents(
            data.agents.as_ref()?,
            data.default_agent_id.clone(),
            data.addable_agents.clone(),
        )
    }

    /// A list of agents as `ready` and `agents.changed` give it, admitted;
    /// one the Mac cannot rely on is logged with its cause and numbers, and
    /// the one the Mac had stays.
    pub(super) fn admitted_agents(
        &self,
        listed: &[orbit_core::wire::hub::HubAgent],
        default_agent_id: Option<String>,
        addable: Option<Vec<String>>,
    ) -> Option<orbit_core::agents::AgentRoster> {
        let agents = listed
            .iter()
            .map(|agent| orbit_core::agents::Agent {
                id: agent.id.clone(),
                name: agent.name.clone(),
                voice: Some(agent.capabilities.iter().any(|name| name == "voice")),
                service: agent.service.clone(),
                description: agent.description.clone(),
                color: agent.color.clone(),
                removable: agent.removable.unwrap_or(false),
                account: agent
                    .account
                    .as_deref()
                    .map(orbit_core::agents::AgentAccount::from_wire),
                account_email: agent.account_email.clone(),
                plan: agent.plan.clone(),
                limit: agent
                    .limit
                    .as_ref()
                    .map(|limit| orbit_core::agents::AgentLimit {
                        until_ms: limit.until_ms,
                    }),
                usage: agent.usage.clone(),
            })
            .collect();
        match orbit_core::agents::AgentRoster::admit(agents, default_agent_id) {
            Ok(roster) => Some(roster.with_addable(addable)),
            Err(refusal) => {
                let numbers = refusal
                    .numbers()
                    .into_iter()
                    .map(|(key, value)| (key, value.to_string()))
                    .collect::<Vec<_>>();
                let mut fields = vec![("cause", refusal.code())];
                fields.extend(numbers.iter().map(|(key, value)| (*key, value.as_str())));
                self.log("hub_agents_refused", &fields);
                self.settings.hub_agents.clone()
            }
        }
    }
}
