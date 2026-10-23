//! The assistant's folders through the daemon: Ajustes adds and removes them
//! through the helper's panel, and Deimos's `files_*` requests go through the
//! real `device_request`, with a synthetic helper answering each step.
//!
//! Everything here crosses the JSON boundaries — the app's command, the
//! helper's lines, the hub's frames — so the cases say what the Mac does, not
//! how the daemon is built.
use crate::daemon::audio::tests::fixture;
use crate::daemon::{Runtime, WorkResult};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

const PROJECTS: &str = "/Users/ejemplo/Proyectos";
const SECRETS: &str = "/Users/ejemplo/Privado/claves.txt";
const BOOKMARK: &str = "Ym9va21hcmstcHJveWVjdG9z";

struct Mac {
    directory: tempfile::TempDir,
    runtime: Runtime,
    helper: mpsc::Receiver<Value>,
    frames: mpsc::Receiver<Value>,
}

async fn mac() -> Result<Mac> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, helper) = fixture(directory.path())?;
    let (hub, frames) = mpsc::channel(64);
    runtime.link.outbound.install(hub);
    Ok(Mac {
        directory,
        runtime,
        helper,
        frames,
    })
}

impl Mac {
    /// The next order the helper received, past the periodic `status`.
    fn helper_command(&mut self, expected: &str) -> Result<Value> {
        loop {
            let command = self
                .helper
                .try_recv()
                .with_context(|| format!("the helper never received «{expected}»"))?;
            if command["command"] == "status" {
                continue;
            }
            if command["command"] != expected {
                bail!("the helper received {command} instead of «{expected}»");
            }
            return Ok(command);
        }
    }

    fn no_helper_command(&mut self, unexpected: &str) -> Result<()> {
        while let Ok(command) = self.helper.try_recv() {
            if command["command"] == unexpected {
                bail!("the helper received «{unexpected}»: {command}");
            }
        }
        Ok(())
    }

    async fn answer(&mut self, command: &Value, result: Value) -> Result<()> {
        self.runtime
            .native_event(
                json!({"v":1,"event":"response","id":command["id"],"ok":true,
                "result":result}),
            )
            .await
    }

    async fn refuse(&mut self, command: &Value, code: &str, detail: Value) -> Result<()> {
        self.runtime
            .native_event(
                json!({"v":1,"event":"response","id":command["id"],"ok":false,
                "error":{"code":code,"message":"Negativa sintética.","detail":detail}}),
            )
            .await
    }

    async fn command(&mut self, body: Value) -> Result<()> {
        let command: orbit_core::protocol::Command = serde_json::from_value(body)?;
        self.runtime.command(command).await
    }

    /// «Añadir carpeta», the panel choosing `path`, and the check after it.
    async fn add_folder(&mut self, path: &str, bookmark: &str) -> Result<String> {
        self.command(json!({"command":"assistant_folder_add"}))
            .await?;
        let pick = self.helper_command("assistant_folder_pick")?;
        self.answer(
            &pick,
            json!({"state":"chosen","path":path,"bookmark_base64":bookmark}),
        )
        .await?;
        let check = self.helper_command("assistant_folders")?;
        let folders = check["folders"]
            .as_array()
            .context("the check carries no folders")?;
        let id = folders
            .iter()
            .find(|folder| folder["bookmark_base64"] == bookmark)
            .and_then(|folder| folder["id"].as_str())
            .context("the new folder is not in the check")?
            .to_owned();
        let reports: Vec<Value> = folders
            .iter()
            .map(|folder| {
                json!({"state":"accessible","id":folder["id"],
                    "path": if folder["id"] == id.as_str() { path.to_owned() }
                        else { format!("/Users/ejemplo/Otra-{}", folder["id"].as_str().unwrap_or("")) }})
            })
            .collect();
        self.answer(&check, json!({ "folders": reports })).await?;
        Ok(id)
    }

    async fn request(&mut self, id: &str, capability: &str, parameters: Value) -> Result<()> {
        self.runtime
            .device_request_raw(&json!({
                "request_id": id, "capability": capability, "parameters": parameters
            }))
            .await
    }

    /// The device reply the hub received for `request_id`.
    fn reply(&mut self, request_id: &str) -> Result<Value> {
        loop {
            let frame = self
                .frames
                .try_recv()
                .with_context(|| format!("no reply reached the hub for {request_id}"))?;
            if frame["method"] == "device.reply" && frame["params"]["request_id"] == request_id {
                return Ok(frame["params"].clone());
            }
        }
    }

    /// The step that resolves `path` among the folders, answered with
    /// `requested` and every folder open where it was chosen.
    async fn resolve(&mut self, folders: &[(&str, &str)], requested: Value) -> Result<Value> {
        let check = self.helper_command("assistant_folders")?;
        let reports: Vec<Value> = folders
            .iter()
            .map(|(id, path)| json!({"state":"accessible","id":id,"path":path}))
            .collect();
        self.answer(&check, json!({"folders":reports,"requested":requested}))
            .await?;
        Ok(check)
    }

    fn folders_topic(&self) -> Result<Value> {
        let orbit_core::topics::TopicData::Control(knowledge) =
            crate::daemon::topic(&self.runtime, orbit_core::topics::TopicId::Control)
        else {
            bail!("the control topic is not the control topic");
        };
        Ok(serde_json::to_value(knowledge)?["value"]["folders"].clone())
    }
}

fn found(path: &str, kind: &str, size: u64) -> Value {
    json!({"state":"found","path":path,"kind":kind,"size_bytes":size,"modified_ms":1_790_200_800_000_i64})
}

async fn mac_with_projects() -> Result<(Mac, String)> {
    let mut mac = mac().await?;
    let id = mac.add_folder(PROJECTS, BOOKMARK).await?;
    Ok((mac, id))
}

#[tokio::test]
async fn a_folder_chosen_in_the_panel_is_kept_encrypted_and_shown_accessible() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let reader = crate::daemon::store_actor::open_reader(
        &crate::config::Paths::resolve(Some(mac.directory.path().to_owned()))?.store,
        [0x42; 32],
    )?;
    let kept = reader.load_assistant_folders()?;
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].path, PROJECTS);
    assert_eq!(kept[0].bookmark_base64, BOOKMARK);
    assert_eq!(kept[0].id, id);
    assert_eq!(
        mac.folders_topic()?,
        json!({"state":"present","as_of_ms":null,"value":[
            {"id":id,"path":PROJECTS,"access":{"state":"accessible"}}]})
    );
    // Choosing it again changes nothing; cancelling the panel changes nothing.
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    mac.answer(
        &pick,
        json!({"state":"chosen","path":PROJECTS,"bookmark_base64":"b3RybyBtYXJjYWRvcg=="}),
    )
    .await?;
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    mac.answer(&pick, json!({"state":"cancelled"})).await?;
    assert_eq!(reader.load_assistant_folders()?.len(), 1);
    // «Quitar» forgets it.
    mac.command(json!({"command":"assistant_folder_remove","id":id}))
        .await?;
    assert_eq!(reader.load_assistant_folders()?, Vec::new());
    assert_eq!(mac.folders_topic()?["value"], json!([]));
    Ok(())
}

#[tokio::test]
async fn without_folders_a_file_request_says_there_are_none() -> Result<()> {
    let mut mac = mac().await?;
    mac.request("no-folders", "files_read", json!({"path":SECRETS}))
        .await?;
    let reply = mac.reply("no-folders")?;
    assert_eq!(reply["refusal"]["code"], "no_folders", "{reply}");
    mac.no_helper_command("assistant_folders")?;
    Ok(())
}

#[tokio::test]
async fn a_path_outside_the_folders_is_refused_with_it_and_the_folders() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    mac.request("outside", "files_read", json!({"path":SECRETS}))
        .await?;
    let check = mac
        .resolve(&[(&id, PROJECTS)], found(SECRETS, "file", 12))
        .await?;
    assert_eq!(check["path"], SECRETS);
    let reply = mac.reply("outside")?;
    assert_eq!(
        reply["refusal"],
        json!({"code":"outside_folders","path":SECRETS,"resolved":null,
            "folders":[PROJECTS],"retry":{"kind":"after_fix"}}),
        "{reply}"
    );
    mac.no_helper_command("assistant_files_read")?;
    Ok(())
}

#[tokio::test]
async fn a_link_that_leads_outside_is_refused_with_where_it_leads() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let link = "/Users/ejemplo/Proyectos/atajo.txt";
    mac.request("link", "files_read", json!({"path":link}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(SECRETS, "file", 12))
        .await?;
    let reply = mac.reply("link")?;
    assert_eq!(reply["refusal"]["code"], "outside_folders", "{reply}");
    assert_eq!(reply["refusal"]["path"], link);
    assert_eq!(reply["refusal"]["resolved"], SECRETS);
    assert_eq!(reply["refusal"]["folders"], json!([PROJECTS]));
    mac.no_helper_command("assistant_files_read")?;
    Ok(())
}

#[tokio::test]
async fn dot_dot_out_of_a_folder_is_refused_without_saying_where_it_leads() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let climbing = "/Users/ejemplo/Proyectos/../Privado/claves.txt";
    mac.request("climb", "files_read", json!({"path":climbing}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(SECRETS, "file", 12))
        .await?;
    let reply = mac.reply("climb")?;
    assert_eq!(reply["refusal"]["code"], "outside_folders", "{reply}");
    assert_eq!(reply["refusal"]["path"], climbing);
    // Written outside the folders: where it leads would say it exists.
    assert_eq!(reply["refusal"]["resolved"], Value::Null);
    mac.no_helper_command("assistant_files_read")?;
    Ok(())
}

#[tokio::test]
async fn a_pdf_over_its_ceiling_is_refused_with_both_sizes_before_any_read() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let manual = "/Users/ejemplo/Proyectos/manual.pdf";
    let size = orbit_core::assistant_files::MAX_PDF_BYTES + 1;
    mac.request("large", "files_read", json!({"path":manual}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(manual, "file", size))
        .await?;
    let reply = mac.reply("large")?;
    assert_eq!(
        reply["refusal"],
        json!({"code":"file_too_large","path":manual,"content":"pdf","bytes":size,
            "max_bytes":orbit_core::assistant_files::MAX_PDF_BYTES,"retry":{"kind":"after_fix"}}),
        "{reply}"
    );
    mac.no_helper_command("assistant_files_read")?;
    Ok(())
}

#[tokio::test]
async fn a_pdf_is_read_as_its_pages_text_with_where_to_go_on() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let manual = "/Users/ejemplo/Proyectos/manual.pdf";
    mac.request("pdf", "files_read", json!({"path":manual,"after_page":2}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(manual, "file", 120_000))
        .await?;
    let read = mac.helper_command("assistant_files_read")?;
    assert_eq!(read["path"], manual);
    assert_eq!(read["folder"]["bookmark_base64"], BOOKMARK);
    assert_eq!(
        read["read"],
        json!({"as":"pdf","after_page":2,"after_index":0,
            "max_bytes":orbit_core::assistant_files::MAX_TEXT_PAGE_BYTES,
            "max_pages":orbit_core::assistant_files::MAX_PDF_PAGES_PER_ANSWER})
    );
    mac.answer(
        &read,
        json!({"content":"pdf","pages":[{"page":3,"text":"Capítulo tres."},{"page":4,"text":""}],
            "page_count":12,"after_page":2,"after_index":0,"next_after_page":4,"next_after_index":0}),
    )
    .await?;
    let reply = mac.reply("pdf")?;
    assert_eq!(
        reply["result"],
        json!({"path":manual,"size_bytes":120_000,"modified_ms":1_790_200_800_000_i64,
            "content":"pdf","pages":[{"page":3,"text":"Capítulo tres."},{"page":4,"text":""}],
            "page_count":12,"after_page":2,"after_index":0,"next_after_page":4,"next_after_index":0}),
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn a_text_file_is_read_a_page_at_a_time() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("text", "files_read", json!({"path":plan,"after_index":10}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(plan, "file", 300_000))
        .await?;
    let read = mac.helper_command("assistant_files_read")?;
    assert_eq!(
        read["read"],
        json!({"as":"text","after_index":10,
            "max_bytes":orbit_core::assistant_files::MAX_TEXT_PAGE_BYTES})
    );
    mac.answer(
        &read,
        json!({"content":"text","text":"El presupuesto.","after_index":10,
            "next_after_index":25,"size_bytes":300_000}),
    )
    .await?;
    let reply = mac.reply("text")?;
    assert_eq!(reply["result"]["content"], "text", "{reply}");
    assert_eq!(reply["result"]["text"], "El presupuesto.");
    assert_eq!(reply["result"]["next_after_index"], 25);
    Ok(())
}

#[tokio::test]
async fn an_image_travels_as_a_capture_and_its_reply_names_it() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let (address, mut puts, release) =
        crate::daemon::audio::tests::actor::capture_hub(false).await?;
    mac.runtime.settings.hub_url = Some(format!("http://{address}"));
    mac.runtime.link.token = Some("synthetic device credential".to_owned());
    let (completions_tx, mut completions) = mpsc::channel::<WorkResult>(16);
    mac.runtime.work_events = completions_tx;
    let photo = "/Users/ejemplo/Proyectos/foto.heic";
    mac.request("image", "files_read", json!({"path":photo}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(photo, "file", 4_000_000))
        .await?;
    let read = mac.helper_command("assistant_files_read")?;
    assert_eq!(
        read["read"],
        json!({"as":"image","max_png_bytes":orbit_core::sources::MAX_SOURCE_BYTES,
            "max_width":orbit_core::sources::MAX_WINDOW_WIDTH,
            "max_height":orbit_core::sources::MAX_WINDOW_HEIGHT})
    );
    let png = two_pixel_png()?;
    mac.answer(
        &read,
        json!({"content":"image",
            "png_base64":base64::engine::general_purpose::STANDARD.encode(&png),
            "width":2,"height":1,"source_width":4032,"source_height":3024}),
    )
    .await?;
    let put = tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("no capture upload")?;
    assert!(put.path.starts_with("/v1/captures/"), "{}", put.path);
    assert_eq!(put.body, png);
    release.notify_one();
    let completion = tokio::time::timeout(Duration::from_secs(10), completions.recv())
        .await?
        .context("the upload never finished")?;
    mac.runtime.work_event(completion).await?;
    let reply = mac.reply("image")?;
    let result = &reply["result"];
    assert_eq!(result["content"], "image", "{reply}");
    assert_eq!(result["mime_type"], "image/png");
    assert_eq!(result["width"], 2);
    assert_eq!(result["source_width"], 4032);
    assert_eq!(
        format!(
            "/v1/captures/{}",
            result["capture_id"].as_str().unwrap_or("")
        ),
        put.path
    );
    assert_eq!(
        result["sha256"],
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&png))
    );
    Ok(())
}

fn two_pixel_png() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()?
            .write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
    }
    Ok(bytes)
}

#[tokio::test]
async fn a_folder_whose_bookmark_no_longer_opens_says_why_in_ajustes_and_to_deimos() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let cause = json!({"kind":"bookmark_unresolvable","domain":"NSCocoaErrorDomain","code":4});
    mac.request(
        "stale",
        "files_read",
        json!({"path":"/Users/ejemplo/Proyectos/plan.md"}),
    )
    .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.answer(
        &check,
        json!({"folders":[{"state":"unavailable","id":id,"cause":cause}],
            "requested":{"state":"not_found"}}),
    )
    .await?;
    let reply = mac.reply("stale")?;
    assert_eq!(
        reply["refusal"],
        json!({"code":"folder_unavailable","path":PROJECTS,"cause":cause,
            "retry":{"kind":"after_fix"}}),
        "{reply}"
    );
    assert_eq!(
        mac.folders_topic()?["value"],
        json!([{"id":id,"path":PROJECTS,"access":{"state":"unavailable","cause":cause}}])
    );
    Ok(())
}

#[tokio::test]
async fn listing_without_a_path_names_the_folders_and_their_state() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    mac.request("roots", "files_list", json!({})).await?;
    let check = mac.helper_command("assistant_folders")?;
    assert!(check.get("path").is_none(), "{check}");
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS}]}),
    )
    .await?;
    let reply = mac.reply("roots")?;
    assert_eq!(
        reply["result"],
        json!({"entries":[{"name":PROJECTS,"kind":"folder"}],"total":1,"after_index":0}),
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn a_listing_inside_a_folder_pages_its_entries() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    mac.request(
        "list",
        "files_list",
        json!({"path":PROJECTS,"after_index":200}),
    )
    .await?;
    mac.resolve(&[(&id, PROJECTS)], found(PROJECTS, "folder", 0))
        .await?;
    let list = mac.helper_command("assistant_files_list")?;
    assert_eq!(list["after_index"], 200);
    assert_eq!(
        list["max_entries"],
        orbit_core::assistant_files::MAX_LIST_ENTRIES
    );
    mac.answer(
        &list,
        json!({"entries":[{"name":"plan.md","kind":"file","size_bytes":2048}],
            "total":201,"after_index":200}),
    )
    .await?;
    let reply = mac.reply("list")?;
    assert_eq!(
        reply["result"],
        json!({"path":PROJECTS,"entries":[{"name":"plan.md","kind":"file","size_bytes":2048}],
            "total":201,"after_index":200}),
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn a_search_walks_every_open_folder_with_its_ceilings() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    mac.request(
        "search",
        "files_search",
        json!({"query":"presupuesto","content":true}),
    )
    .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS}]}),
    )
    .await?;
    let search = mac.helper_command("assistant_files_search")?;
    assert_eq!(search["query"], "presupuesto");
    assert_eq!(search["content"], true);
    assert_eq!(search["roots"][0]["path"], PROJECTS);
    assert_eq!(
        search["limits"]["max_results"],
        orbit_core::assistant_files::MAX_SEARCH_RESULTS
    );
    mac.answer(
        &search,
        json!({"results":[{"path":"/Users/ejemplo/Proyectos/plan.md","kind":"file","line":3,
                "snippet":"El presupuesto."}],
            "visited":12,"files_read":4,"bytes_read":8192,"partial_files":0,
            "stopped":{"cause":"deadline","limit":10000}}),
    )
    .await?;
    let reply = mac.reply("search")?;
    assert_eq!(reply["result"]["results"][0]["line"], 3, "{reply}");
    assert_eq!(
        reply["result"]["stopped"],
        json!({"cause":"deadline","limit":10000})
    );
    Ok(())
}

#[tokio::test]
async fn a_file_the_helper_finds_changed_is_refused_with_its_path() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("changed", "files_read", json!({"path":plan}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(plan, "file", 10))
        .await?;
    let read = mac.helper_command("assistant_files_read")?;
    mac.refuse(
        &read,
        "assistant_file_unavailable",
        json!({"cause":{"kind":"changed"}}),
    )
    .await?;
    let reply = mac.reply("changed")?;
    assert_eq!(
        reply["refusal"],
        json!({"code":"file_unavailable","path":plan,"cause":{"kind":"changed"},
            "retry":{"kind":"after_fix"}}),
        "{reply}"
    );
    Ok(())
}

#[tokio::test]
async fn a_helper_step_that_never_answers_frees_the_bridge_with_both_numbers() -> Result<()> {
    let (mut mac, _id) = mac_with_projects().await?;
    mac.request(
        "silent",
        "files_read",
        json!({"path":"/Users/ejemplo/Proyectos/plan.md"}),
    )
    .await?;
    mac.helper_command("assistant_folders")?;
    let Some(request) = mac.runtime.files.request.as_mut() else {
        bail!("no request in flight");
    };
    request.started = std::time::Instant::now()
        .checked_sub(Duration::from_millis(
            orbit_core::assistant_files::HELPER_REQUEST_TIMEOUT_MS + 1,
        ))
        .context("the clock cannot go back that far")?;
    mac.runtime.check_files_timeout();
    let reply = mac.reply("silent")?;
    assert_eq!(reply["refusal"]["code"], "timeout", "{reply}");
    assert_eq!(reply["refusal"]["operation"], "files");
    assert_eq!(
        reply["refusal"]["deadline_ms"],
        orbit_core::assistant_files::HELPER_REQUEST_TIMEOUT_MS
    );
    assert!(mac.runtime.assistant.bridge_request.is_none());
    Ok(())
}

#[tokio::test]
async fn a_helper_that_restarts_mid_request_refuses_it_and_forgets_what_it_knew() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    mac.request(
        "restart",
        "files_read",
        json!({"path":"/Users/ejemplo/Proyectos/plan.md"}),
    )
    .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.runtime.files_helper_restarted();
    let reply = mac.reply("restart")?;
    assert_eq!(reply["refusal"]["code"], "read_interrupted", "{reply}");
    assert_eq!(reply["refusal"]["cause"]["kind"], "helper_restarted");
    assert_eq!(
        mac.folders_topic()?["value"],
        json!([{"id":id,"path":PROJECTS,"access":{"state":"unknown"}}])
    );
    // The old helper's late answer reaches nobody.
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS}],
            "requested":found("/Users/ejemplo/Proyectos/plan.md","file",1)}),
    )
    .await?;
    mac.no_helper_command("assistant_files_read")?;
    assert!(mac.frames.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn the_privacy_stop_answers_a_request_once_and_its_late_reply_is_dropped() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("stopped", "files_read", json!({"path":plan}))
        .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    let reply = mac.reply("stopped")?;
    assert_eq!(reply["refusal"]["code"], "privacy_stopped", "{reply}");
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS}],
            "requested":found(plan,"file",1)}),
    )
    .await?;
    mac.no_helper_command("assistant_files_read")?;
    while let Ok(frame) = mac.frames.try_recv() {
        assert_ne!(
            frame["params"]["request_id"], "stopped",
            "answered twice: {frame}"
        );
    }
    Ok(())
}

/// The entries the translator's suite proves it sends
/// (`contracts/fixtures/device-bridge-request-v1.json`) reach the real handler
/// typed and start the folders step with their path.
#[tokio::test]
async fn the_shared_request_contract_starts_the_folders_step() -> Result<()> {
    let document: Value = serde_json::from_str(include_str!(
        "../../../../../contracts/fixtures/device-bridge-request-v1.json"
    ))?;
    let entries: Vec<Value> = document["requests"]
        .as_array()
        .context("no requests in the contract")?
        .iter()
        .filter(|entry| {
            entry["capability"]
                .as_str()
                .is_some_and(|name| name.starts_with("files_"))
        })
        .cloned()
        .collect();
    assert_eq!(entries.len(), 3);
    for (index, entry) in entries.iter().enumerate() {
        let (mut mac, _id) = mac_with_projects().await?;
        let request_id = format!("contract-{index}");
        mac.request(
            &request_id,
            entry["capability"].as_str().unwrap_or(""),
            entry["parameters"].clone(),
        )
        .await?;
        let check = mac.helper_command("assistant_folders")?;
        assert_eq!(check["path"], entry["parameters"]["path"], "{entry}");
    }
    Ok(())
}

mod lifecycle;
