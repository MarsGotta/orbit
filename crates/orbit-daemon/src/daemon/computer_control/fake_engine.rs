//! One synthetic MCP peer for catalogue, permissions, captures and desktop files.
use super::*;

// A declared tool at the production name ceiling, for history budget tests.
pub(super) const LONG_TOOL: &str = "fixture_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
use base64::Engine as _;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Default)]
pub(super) struct EngineRecord {
    pub calls: Arc<Mutex<Vec<Value>>>,
    pub catalogue_reads: Arc<AtomicUsize>,
    pub full_screen_frame_bytes: Arc<AtomicUsize>,
}

impl EngineRecord {
    pub fn calls_of(&self, tool: &str) -> Vec<Value> {
        self.calls
            .lock()
            .expect("fixture lock")
            .iter()
            .filter(|call| call["name"] == tool)
            .cloned()
            .collect()
    }
}

#[derive(Clone)]
pub(super) enum DesktopFile {
    Png(Arc<Vec<u8>>),
    Sparse(u64),
    Nothing,
}

pub(super) struct FakeEngine {
    pub tools: Value,
    pub windows: Value,
    pub png: String,
    pub window_dimensions: (u32, u32),
    pub desktop_dimensions: (u32, u32),
    pub desktop_png: Option<String>,
    pub desktop_file: Option<DesktopFile>,
    pub hang_input: bool,
    pub timeout: Duration,
    pub record: EngineRecord,
    /// The window's tree, answered to a `get_window_state` without a picture.
    pub tree: Option<String>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        let tools = tools_named(&[
            "list_apps",
            "list_windows",
            "get_window_state",
            "get_desktop_state",
            "launch_app",
            "bring_to_front",
            "click",
            "type_text",
            "press_key",
            "scroll",
            INVENTED_TOOL,
        ]);
        Self {
            tools: json!(tools),
            windows: json!({"windows":[{"window_id":22,"pid":11,
                "app_name":"Fixture","title":"Fixture","z_index":1}]}),
            png: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
            window_dimensions: (1, 1),
            desktop_dimensions: (1, 1),
            desktop_png: None,
            desktop_file: None,
            hang_input: false,
            timeout: Duration::from_millis(300),
            record: EngineRecord::default(),
            tree: None,
        }
    }
}

impl FakeEngine {
    pub async fn connect(self) -> Result<crate::cua::OwnedCua> {
        let (transport, peer) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(transport);
        let mut client = CuaClient::from_transport(BufReader::new(reader), writer, self.timeout)?;
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(peer);
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                assert!(line.len() < 16 * 1024);
                let request: Value = serde_json::from_str(&line).expect("fixture request JSON");
                let result = match request["method"].as_str() {
                    Some("notifications/initialized") => continue,
                    Some("initialize") => json!({"protocolVersion":"2025-06-18",
                        "serverInfo":{"name":"cua-driver","version":crate::cua::VERIFIED_DRIVER_VERSION},
                        "capabilities":{"tools":{}}}),
                    Some("tools/list") => {
                        self.record.catalogue_reads.fetch_add(1, Ordering::SeqCst);
                        json!({"tools":self.tools})
                    }
                    Some("tools/call") => {
                        let call = &request["params"];
                        let name = call["name"].as_str().expect("fixture tool");
                        self.record
                            .calls
                            .lock()
                            .expect("fixture lock")
                            .push(call.clone());
                        assert!(self.record.calls.lock().expect("fixture lock").len() < 32);
                        let mut content = json!([]);
                        let structured = match name {
                            "list_apps" => json!({"apps":[{"pid":11,"name":"Fixture"}]}),
                            "list_windows" => self.windows.clone(),
                            "get_window_state" | "get_desktop_state" => {
                                if name == "get_desktop_state" {
                                    if let Some(file) = &self.desktop_file {
                                        let path = call["arguments"]["screenshot_out_file"]
                                            .as_str()
                                            .map(std::path::PathBuf::from);
                                        match (file, &path) {
                                            (DesktopFile::Png(bytes), Some(path)) => {
                                                std::fs::write(path, bytes.as_slice())
                                                    .expect("fixture file")
                                            }
                                            (DesktopFile::Sparse(length), Some(path)) => {
                                                std::fs::File::create(path)
                                                    .and_then(|file| file.set_len(*length))
                                                    .expect("fixture sparse file")
                                            }
                                            (DesktopFile::Nothing, _) | (_, None) => {}
                                        }
                                        json!({"screenshot_file_path":path.map(|p|p.display().to_string()),
                                            "screenshot_width":self.desktop_dimensions.0,"screenshot_height":self.desktop_dimensions.1})
                                    } else {
                                        if let Some(png) = &self.desktop_png {
                                            content = json!([{"type":"image","mimeType":"image/png","data":png}]);
                                        }
                                        json!({"status":"ok","tool":name,"snapshot_id":"general-1",
                                            "screenshot_width":self.desktop_dimensions.0,"screenshot_height":self.desktop_dimensions.1})
                                    }
                                } else if let Some(tree) = self
                                    .tree
                                    .as_ref()
                                    .filter(|_| call["arguments"]["include_screenshot"] == false)
                                {
                                    json!({"status":"ok","tool":name,"snapshot_id":"general-2",
                                        "tree_markdown":tree})
                                } else {
                                    content = json!([{"type":"image","mimeType":"image/png","data":self.png}]);
                                    if call["arguments"]["query"] == ENORMOUS_TREE {
                                        json!({"tree_markdown":"- AXStaticText = \"x\"\n".repeat(2_000)})
                                    } else {
                                        json!({"status":"ok","tool":name,"snapshot_id":"general-1",
                                            "screenshot_width":self.window_dimensions.0,"screenshot_height":self.window_dimensions.1})
                                    }
                                }
                            }
                            "launch_app" | "bring_to_front" | "click" | "type_text"
                            | "press_key" | "scroll" | INVENTED_TOOL | LONG_TOOL => {
                                if self.hang_input {
                                    continue;
                                }
                                json!({"status":"ok","tool":name,"attempted":true})
                            }
                            other => panic!("unexpected fixture tool: {other}"),
                        };
                        json!({"content":content,"structuredContent":structured,"isError":false})
                    }
                    other => panic!("unexpected fixture method: {other:?}"),
                };
                let mut bytes = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .expect("fixture response");
                bytes.push(b'\n');
                if request["params"]["name"] == "get_desktop_state" {
                    self.record
                        .full_screen_frame_bytes
                        .store(bytes.len(), Ordering::SeqCst);
                }
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });
        client.initialize().await?;
        Ok(client.into_owned())
    }

    pub fn with_png(png: &[u8]) -> Self {
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        Self {
            tools: Value::Array(tools_named(&[
                "list_apps",
                "list_windows",
                "get_browser_state",
                "get_window_state",
                "get_desktop_state",
            ])),
            png: encoded.clone(),
            desktop_png: Some(encoded),
            window_dimensions: (2, 1),
            desktop_dimensions: (2, 1),
            timeout: Duration::from_secs(5),
            ..Self::default()
        }
    }
}

pub(super) const REAL_CATALOGUE_BYTES: usize = 147_909;

/// One tool declared the way a real engine declares it: a description and an
/// input schema with documented, constrained properties.
pub(super) fn realistic_tool(index: usize) -> Value {
    let mut properties = serde_json::Map::new();
    for field in 0..10 {
        properties.insert(
            format!("parameter_{field}"),
            json!({
                "type": "string",
                "title": format!("Parámetro {field}"),
                "description": format!(
                    "Argumento {field} de la herramienta {index}, descrito por el motor con el \
                     detalle que necesita quien la llama para no adivinar su forma ni su unidad."
                ),
                "enum": ["ninguno", "primero", "segundo", "tercero"],
            }),
        );
    }
    json!({
        "name": format!("engine_tool_{index:03}"),
        "description": format!(
            "Herramienta {index} del motor de control, con su descripción completa tal como el \
             motor la publica en su catálogo."
        ),
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["parameter_0", "parameter_1"],
            "additionalProperties": false,
        },
    })
}

pub(super) fn tools_named(names: &[&str]) -> Vec<Value> {
    names
        .iter()
        .map(|name| json!({"name":name,"inputSchema":{"type":"object"}}))
        .collect()
}

/// A catalogue of at least `target` bytes, grown one realistic tool at a time.
pub(super) fn catalogue_of(target: usize) -> Result<(Value, usize)> {
    let mut tools = tools_named(&["list_apps", "list_windows", "get_browser_state"]);
    let mut index = 0;
    while serde_json::to_vec(&tools)?.len() < target {
        tools.push(realistic_tool(index));
        index += 1;
    }
    let bytes = serde_json::to_vec(&tools)?.len();
    Ok((Value::Array(tools), bytes))
}
