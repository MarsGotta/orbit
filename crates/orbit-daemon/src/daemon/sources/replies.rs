//! The helper's replies to a source order: a stale one closes its operation
//! with its cause, a refusal keeps only the helper's code, and a result is
//! admitted only as the contract says.
use super::*;

impl Runtime {
    pub(in crate::daemon) async fn source_response(&mut self, event: &Value) -> Result<bool> {
        let Some(command_id) = NativeResponseIdRef::from_value(event)
            .ok()
            .map(|response| response.id)
            .filter(|id| id.starts_with("source-list-") || id.starts_with("source-capture-"))
        else {
            return Ok(false);
        };
        if !self
            .sources
            .operation
            .as_ref()
            .is_some_and(|op| op.command_id == command_id)
        {
            return Ok(true); // Late or duplicate ACK cannot stop unrelated audio or capture again.
        }
        let operation = self
            .sources
            .operation
            .take()
            .context("Source operation missing")?;
        let settlement = self
            .sources
            .reply
            .settle(&operation.ticket, self.state.generation());
        let stale = self.stale_source_reply(&operation, &settlement);
        if let Some(refusal) = stale {
            self.close_source_operation(&operation, SourceStage::Unknown, refusal)
                .await?;
            return Ok(true);
        }
        let response = NativeResponseFields::from_value(event)?;
        if !response.ok {
            // Keep the closed helper code and its facts, never its untrusted message.
            let refusal = match response.error.as_ref() {
                Some(error) => Runtime::bridge_refusal_code(&error.code),
                None => Refusal::HelperReplyInvalid {
                    field: "error".into(),
                    expected: "native_error".into(),
                    received_type: "absent".into(),
                },
            };
            self.close_source_operation(&operation, SourceStage::Failed, refusal)
                .await?;
            return Ok(true);
        }
        let body = response
            .result
            .as_ref()
            .context("response source field=result type=absent")?;
        let result = if let Some(intent) = &operation.intent {
            self.accept_source_content(intent, body)
        } else {
            self.accept_source_inventory(&operation, body)
        };
        if let Err(error) = result {
            self.close_source_operation(
                &operation,
                SourceStage::Failed,
                source_contract_refusal(error, "source.result"),
            )
            .await?;
        }
        Ok(true)
    }

    /// Why a source reply no longer counts: the user's stop, another
    /// generation or conversation, its deadline, or a newer request.
    pub(super) fn stale_source_reply(
        &self,
        operation: &Operation,
        settlement: &Settlement<()>,
    ) -> Option<Refusal> {
        if self.state.privacy_stopped() {
            Some(Refusal::PrivacyStopped { since_ms: None })
        } else if let Settlement::Expired {
            started_generation,
            current_generation,
        } = settlement
        {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::GenerationChanged {
                    started_generation: *started_generation,
                    current_generation: *current_generation,
                },
            })
        } else if operation.session_id != self.state.session_id() {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::ConversationChanged,
            })
        } else if operation.deadline.expired() {
            Some(source_timeout_refusal(operation.deadline))
        } else if matches!(settlement, Settlement::Foreign) {
            Some(Refusal::ReadInterrupted {
                cause: ReadInterruption::SourceReplyReplaced,
            })
        } else {
            None
        }
    }

    pub(super) fn accept_source_inventory(
        &mut self,
        operation: &Operation,
        result: &Value,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(result)?.len();
        if bytes > orbit_core::limits::MAX_WINDOW_INVENTORY_BYTES {
            return Err(Refusal::TooLarge {
                resource: Resource::Source,
                bytes: bytes as u64,
                max_bytes: orbit_core::limits::MAX_WINDOW_INVENTORY_BYTES as u64,
            }
            .into());
        }
        let inventory: WindowInventory =
            decode_result_body("screen_sources", result).map_err(Refusal::helper_reply_invalid)?;
        inventory.validate()?;
        // Conservative: the timer begins when the request was sent, not on receipt.
        let request_started = operation.deadline.started();
        let ttl =
            Duration::from_nanos(inventory.expires_monotonic_ns - inventory.observed_monotonic_ns);
        let deadline = request_started + ttl;
        if Instant::now() >= deadline {
            return Err(source_timeout_refusal(operation.deadline).into());
        }
        self.sources.inventory_deadline = Some(deadline);
        self.state.views.source_inventory = Some(inventory);
        self.source_progress(
            &operation.id,
            SourceStage::Available,
            "Choose a window and format for a single read.",
        );
        Ok(())
    }

    pub(super) fn accept_source_content(
        &mut self,
        intent: &SourceIntent,
        result: &Value,
    ) -> Result<()> {
        let (metadata, bytes) = content::decode(intent, result, now())?;
        let receipt = SourceReceipt {
            intent: intent.clone(),
            metadata,
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        receipt.validate()?;
        self.state.views.source_receipt = Some(receipt);
        self.source_progress(
            &intent.id,
            SourceStage::Validated,
            "Read validated in memory; it is not retained on this Mac.",
        );
        Ok(())
    }

    pub(in crate::daemon) fn remote_source_response(&self, command_id: &str) -> bool {
        self.sources.operation.as_ref().is_some_and(|operation| {
            operation.command_id == command_id && self.assistant.source_is_remote(&operation.id)
        })
    }
}
