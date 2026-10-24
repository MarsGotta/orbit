//! The MCP exchange with the engine's proxy: the handshake that checks its
//! version and catalogue, one request at a time, and the death of the process
//! named when the transport goes silent.
use super::transport::{ResponseTooLarge, read_json, the_engine_end_is_gone, write_json};
use super::{CuaClient, EngineExit, ORBIT_READ_TOOLS, ToolDescription};
use super::{VERIFIED_DRIVER_VERSION, VERIFIED_MCP_VERSION};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{process::ExitStatus, time::Duration};
use tokio::io::{AsyncBufRead, AsyncWrite};

const MAX_NOTIFICATION_COUNT: usize = 64;
/// How long Orbit waits, once the transport says the engine's end is gone —
/// end of file on what it writes, or a broken pipe on what Orbit writes to it
/// — to learn whether the process it started has died. An exit closes the
/// process's pipes before its status can be collected, so at that instant the
/// status is often not there yet: in half the broken pipes and 86 % of the
/// ends of file measured. Over 44,000 deaths at birth on Linux, with
/// load averages of 44 to 91 and up to sixteen busy loops per CPU, the status
/// arrived at p99 within 43 ms of the report and at worst within 77 ms; 500 ms
/// is over six times that worst case. Linux only: tokio learns of the exit
/// through a pidfd there and through SIGCHLD on macOS, which is unmeasured.
/// It is not a deadline on the engine's work, which the request timeout
/// bounds: a process still running when it expires keeps its own message, so
/// all it costs is that much delay in reporting a live engine that closed its
/// pipes, and a death slower than this to collect is reported as the transport
/// saw it, never as a death that did not happen.
const ENGINE_EXIT_GRACE: Duration = Duration::from_millis(500);

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> CuaClient<R, W> {
    pub async fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            bail!("La sesión CUA ya está inicializada");
        }
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion":VERIFIED_MCP_VERSION, "capabilities":{},
                    "clientInfo":{"name":"orbit", "version":env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        let Some(actual_driver_version) = result["serverInfo"]["version"].as_str() else {
            self.poison();
            bail!("Versión de cua-driver ausente; se esperaba {VERIFIED_DRIVER_VERSION}");
        };
        if actual_driver_version != VERIFIED_DRIVER_VERSION {
            self.poison();
            bail!(
                "Versión de cua-driver {actual_driver_version}; se esperaba {VERIFIED_DRIVER_VERSION}"
            );
        }
        if result["protocolVersion"] != VERIFIED_MCP_VERSION {
            self.poison();
            bail!("Versión MCP no verificada; se esperaba {VERIFIED_MCP_VERSION}");
        }
        self.notify("notifications/initialized").await?;
        let tools = self.request("tools/list", json!({})).await?;
        if tools
            .get("nextCursor")
            .is_some_and(|value| !value.is_null())
        {
            self.poison();
            bail!("Catálogo CUA paginado no compatible con este adaptador");
        }
        let descriptions: Vec<ToolDescription> = serde_json::from_value(tools["tools"].clone())?;
        if descriptions.len() > orbit_core::computer_control::MAX_ENGINE_TOOLS {
            self.poison();
            bail!("Catálogo CUA demasiado grande");
        }
        // The catalogue's own bound, applied where the catalogue actually
        // arrives. An engine that answers with something absurd is refused
        // here, by name, instead of being kept and failing later on somebody
        // else's limit.
        let catalogue_bytes = serde_json::to_vec(&descriptions)?.len();
        if catalogue_bytes > orbit_core::computer_control::MAX_ENGINE_CATALOGUE_BYTES {
            self.poison();
            bail!(
                "El catálogo del motor ocupa {catalogue_bytes} bytes y supera su límite de {} bytes; un catálogo no se entrega recortado",
                orbit_core::computer_control::MAX_ENGINE_CATALOGUE_BYTES
            );
        }
        // Kept whole, on purpose: the catalogue Deimos is offered is the one the
        // engine declares, so a newer engine grows a tool without touching Orbit.
        self.tools = descriptions;
        if !ORBIT_READ_TOOLS
            .iter()
            .all(|name| self.tools.iter().any(|tool| tool.name == *name))
        {
            self.poison();
            bail!("El driver no ofrece las herramientas de lectura verificadas");
        }
        self.initialized = true;
        Ok(())
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        if !self.alive {
            bail!("La conexión CUA está cerrada; no se reenvían operaciones");
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("Secuencia MCP agotada")?;
        let id = self.next_id;
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let timeout = self.timeout;
        // Cancellation drops this future before the match below. Leave the
        // transport poisoned in that case, so buffered responses cannot be reused.
        self.alive = false;
        let operation = async {
            write_json(&mut self.writer, &message).await?;
            for _ in 0..MAX_NOTIFICATION_COUNT {
                let response = read_json(&mut self.reader).await?;
                if response["jsonrpc"] != "2.0" {
                    bail!("Respuesta MCP inválida");
                }
                if response.get("method").is_some() {
                    if response.get("id").is_some() {
                        bail!("CUA solicitó una operación de cliente no autorizada");
                    }
                    match response["method"].as_str() {
                        Some("notifications/progress" | "notifications/message") => continue,
                        _ => {
                            bail!("La sesión MCP cambió; es necesario volver a validar su catálogo")
                        }
                    }
                }
                if response["id"].as_u64() != Some(id) {
                    bail!("Identidad de respuesta MCP incorrecta");
                }
                if response.get("error").is_some() {
                    bail!("CUA rechazó la petición MCP");
                }
                return response
                    .get("result")
                    .cloned()
                    .context("Respuesta MCP sin resultado");
            }
            bail!("Exceso de notificaciones MCP");
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(value)) => {
                self.alive = true;
                Ok(value)
            }
            // The answer was read to its end: the connection is in step, and
            // the engine, which answered, is not killed for its size.
            // It is taken as this request's answer. Had it been a
            // notification instead, the real answer would reach the next
            // request with the wrong identity, and that request would be
            // refused by name and close the connection: one request lost,
            // none repeated.
            Ok(Err(error)) if error.downcast_ref::<ResponseTooLarge>().is_some() => {
                self.alive = true;
                Err(error)
            }
            Ok(Err(error)) => {
                // Before poisoning, because poisoning kills the process and a
                // killed process cannot be told apart from one that died on
                // its own.
                let error = self
                    .name_the_death_behind_the_silence(error, Sent::Request)
                    .await;
                self.poison();
                Err(error)
            }
            Err(_) => {
                let status = self.check_processes();
                self.poison();
                status?;
                bail!("Tiempo de espera CUA agotado; no se reenvía la petición")
            }
        }
    }

    /// End of file says the engine stopped talking, never why. A driver binary
    /// the system starts and that dies in the act — what macOS makes of a
    /// broken executable, where Linux refuses to start it at all — leaves
    /// nothing behind but that silence. So when the process Orbit started is
    /// already gone, the death is the cause and gets named, with its status.
    /// When it is still running the connection really did close, and the
    /// message stays as it was: no death is attributed that did not happen.
    /// A write that finds the pipe broken is the same silence, found by the
    /// write before the read gets to it. What was in flight decides
    /// what the death can truthfully be called: a death after the engine
    /// answered is named as that, not as a binary that never started.
    async fn name_the_death_behind_the_silence(
        &mut self,
        error: anyhow::Error,
        sent: Sent<'_>,
    ) -> anyhow::Error {
        if let Err(exit) = self.check_processes() {
            if let Some(death) = exit.downcast_ref::<EngineExit>() {
                let status = death.status;
                return exit.context(death_context(status, sent, self.next_id));
            }
            return exit.context(error);
        }
        if !the_engine_end_is_gone(&error) {
            return error;
        }
        let Some(child) = self.child.as_mut() else {
            return error;
        };
        let Ok(Ok(status)) = tokio::time::timeout(ENGINE_EXIT_GRACE, child.wait()).await else {
            return error;
        };
        anyhow::Error::new(EngineExit {
            process: "proxy",
            status,
        })
        .context(death_context(status, sent, self.next_id))
    }

    /// Sent only after `initialize` was answered. A failure keeps its cause in
    /// the message, since what reaches the person is the message alone: the
    /// engine's death with its code, a timeout with its ceiling, or whatever
    /// else the write reported.
    async fn notify(&mut self, method: &str) -> Result<()> {
        match tokio::time::timeout(
            self.timeout,
            write_json(&mut self.writer, &json!({"jsonrpc":"2.0","method":method})),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                // Before poisoning, as for a request: poisoning kills the
                // process, and a killed process looks like one that died.
                let error = self
                    .name_the_death_behind_the_silence(error, Sent::Notification(method))
                    .await;
                self.poison();
                bail!("No se pudo inicializar la sesión CUA: {error:#}")
            }
            Err(_) => {
                self.poison();
                bail!(
                    "No se pudo inicializar la sesión CUA: tiempo de espera agotado tras {} ms al enviar la notificación {method}; no se reenvía",
                    self.timeout.as_millis()
                )
            }
        }
    }

    pub(super) fn poison(&mut self) {
        self.alive = false;
        self.initialized = false;
        self.tools.clear();
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        // The engine goes with the connection: a reconnect starts a fresh one
        // rather than reattaching to a process whose state Orbit cannot see.
        drop(self.engine.take());
    }
}

fn death_context(status: ExitStatus, sent: Sent<'_>, next_id: u64) -> String {
    match sent {
        Sent::Request if next_id > 1 => format!(
            "El motor CUA terminó ({status}) sin responder a esta petición, tras haber respondido a las anteriores; no se reenvía la petición"
        ),
        Sent::Request => format!(
            "El motor CUA terminó nada más arrancar ({status}) sin responder al protocolo MCP; su binario puede estar roto"
        ),
        Sent::Notification(method) => format!(
            "El motor CUA terminó ({status}) sin recibir la notificación {method}, tras haber respondido a las peticiones anteriores; no se reenvía la notificación"
        ),
    }
}

/// What Orbit had just sent when the transport went silent.
#[derive(Clone, Copy)]
enum Sent<'a> {
    /// A request, which the engine owed an answer.
    Request,
    /// A notification, by its method, which is never answered.
    Notification(&'a str),
}
