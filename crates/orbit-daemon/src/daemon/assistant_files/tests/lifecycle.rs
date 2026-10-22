//! The folders across a request's life: a folder removed meanwhile, the
//! request's whole deadline, what a refused choice says on screen, what an
//! outside path reveals, and a search over nested folders.
use super::*;

/// The hub's wait for one non-action bridge answer
/// (`server/orbit_hub/device_bridge.py`, `REQUEST_TIMEOUT_SECONDS`).
const HUB_BRIDGE_WAIT_MS: u64 = 30_000;

fn back_by(mac: &mut Mac, millis: u64) -> Result<()> {
    let Some(request) = mac.runtime.features.files.request.as_mut() else {
        bail!("no request in flight");
    };
    request.started = request
        .started
        .checked_sub(Duration::from_millis(millis))
        .context("the clock cannot go back that far")?;
    Ok(())
}

/// «Quitar» forgets a folder at once: a request whose first step was in
/// flight when the person removed it must not go on to read inside it.
#[tokio::test]
async fn a_folder_removed_while_a_request_resolves_is_not_read_afterwards() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("removed", "files_read", json!({"path":plan}))
        .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.command(json!({"command":"assistant_folder_remove","id":id}))
        .await?;
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS}],
            "requested":found(plan,"file",10)}),
    )
    .await?;
    mac.no_helper_command("assistant_files_read")?;
    let reply = mac.reply("removed")?;
    assert!(
        reply.get("refusal").is_some(),
        "a removed folder answered: {reply}"
    );
    Ok(())
}

/// The step bound says the refusal «still reaches the hub before its 30 s».
/// Each step restarts it, so a request whose two steps take 19 s each still
/// holds the bridge at 38 s, after the hub answered the assistant `no_answer`.
#[tokio::test]
async fn a_two_step_request_is_refused_before_the_hub_stops_waiting() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("slow", "files_read", json!({"path":plan}))
        .await?;
    let step = orbit_core::assistant_files::HELPER_REQUEST_TIMEOUT_MS - 1_000;
    back_by(&mut mac, step)?;
    mac.resolve(&[(&id, PROJECTS)], found(plan, "file", 10))
        .await?;
    mac.helper_command("assistant_files_read")?;
    back_by(&mut mac, step)?;
    assert!(2 * step > HUB_BRIDGE_WAIT_MS);
    mac.runtime.check_files_timeout();
    let reply = mac.reply("slow")?;
    assert_eq!(reply["refusal"]["code"], "timeout", "{reply}");
    Ok(())
}

/// A folder the daemon will not keep (its bookmark is over the ceiling) is
/// refused only with a silent notice: the person chose a folder in the panel
/// and nothing appears, with no reason.
#[tokio::test]
async fn a_chosen_folder_the_daemon_refuses_says_why_on_screen() -> Result<()> {
    let mut mac = mac().await?;
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    let oversized = "A".repeat(orbit_core::assistant_files::MAX_BOOKMARK_BYTES.div_ceil(3) * 4 + 8);
    mac.answer(
        &pick,
        json!({"state":"chosen","path":PROJECTS,"bookmark_base64":oversized}),
    )
    .await?;
    assert_eq!(mac.folders_topic()?["value"], json!([]));
    let notice = mac
        .runtime
        .state
        .durable
        .notice
        .clone()
        .context("no notice at all")?;
    assert_ne!(
        notice.code.class(),
        orbit_core::notice::NoticeClass::Silent,
        "the refusal is never shown: {notice:?}"
    );
    Ok(())
}

/// The same for a panel the helper could not complete (the bookmark could
/// not be made): the helper's code reaches only the log.
#[tokio::test]
async fn a_panel_the_helper_fails_says_why_on_screen() -> Result<()> {
    let mut mac = mac().await?;
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    mac.refuse(&pick, "assistant_folder_unavailable", json!({}))
        .await?;
    let notice = mac
        .runtime
        .state
        .durable
        .notice
        .clone()
        .context("no notice at all")?;
    assert_ne!(
        notice.code.class(),
        orbit_core::notice::NoticeClass::Silent,
        "the failure is never shown: {notice:?}"
    );
    Ok(())
}

/// «Outside the folders, Orbit does not say what exists» (`admit`). A path
/// outside every folder written with `/./` (or `//`, or another case on
/// APFS) resolves to a different string when it exists, and the refusal
/// then names where it leads: whether the file exists, and its true name.
#[tokio::test]
async fn a_path_outside_the_folders_does_not_say_whether_it_exists() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let probing = "/Users/ejemplo/Privado/./claves.txt";
    mac.request("exists", "files_read", json!({"path":probing}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(SECRETS, "file", 12))
        .await?;
    let existing = mac.reply("exists")?;
    let missing_path = "/Users/ejemplo/Privado/./nada.txt";
    mac.request("missing", "files_read", json!({"path":missing_path}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], json!({"state":"not_found"}))
        .await?;
    let missing = mac.reply("missing")?;
    assert_eq!(existing["refusal"]["code"], "outside_folders");
    assert_eq!(missing["refusal"]["code"], "outside_folders");
    assert_eq!(
        existing["refusal"]["resolved"], missing["refusal"]["resolved"],
        "an existing file outside the folders is told apart from a missing one: \
         {existing} vs {missing}"
    );
    Ok(())
}

/// Two folders, one inside the other: a search of every folder walks the
/// inner one twice (the helper walks each root whole), so its matches come
/// back twice and fill `max_results` with duplicates.
#[tokio::test]
async fn a_search_of_nested_folders_walks_each_place_once() -> Result<()> {
    let (mut mac, outer) = mac_with_projects().await?;
    let web = "/Users/ejemplo/Proyectos/Web";
    let inner = mac.add_folder(web, "Ym9va21hcmstd2Vi").await?;
    mac.request("nested", "files_search", json!({"query":"presupuesto"}))
        .await?;
    mac.resolve(&[(&outer, PROJECTS), (&inner, web)], Value::Null)
        .await?;
    let search = mac.helper_command("assistant_files_search")?;
    let roots: Vec<&str> = search["roots"]
        .as_array()
        .context("no roots")?
        .iter()
        .filter_map(|root| root["path"].as_str())
        .collect();
    for root in &roots {
        let covered = roots.iter().any(|other| {
            other != root && root.starts_with(&format!("{}/", other.trim_end_matches('/')))
        });
        assert!(
            !covered,
            "{root} is walked again inside another root: {roots:?}"
        );
    }
    Ok(())
}

/// Removed while the helper reads in it: what it read there is not sent.
#[tokio::test]
async fn a_folder_removed_while_its_file_is_read_does_not_answer_with_it() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("reading", "files_read", json!({"path":plan}))
        .await?;
    mac.resolve(&[(&id, PROJECTS)], found(plan, "file", 10))
        .await?;
    let read = mac.helper_command("assistant_files_read")?;
    mac.command(json!({"command":"assistant_folder_remove","id":id}))
        .await?;
    mac.answer(
        &read,
        json!({"content":"text","text":"contenido inventado","after_index":0,"size_bytes":19}),
    )
    .await?;
    let reply = mac.reply("reading")?;
    assert_eq!(reply["refusal"]["code"], "no_folders", "{reply}");
    assert!(!reply.to_string().contains("contenido inventado"));
    Ok(())
}

/// A search of every folder answers only from the folders still there.
#[tokio::test]
async fn a_search_of_every_folder_drops_what_it_found_in_a_removed_one() -> Result<()> {
    let (mut mac, projects) = mac_with_projects().await?;
    let notes = "/Users/ejemplo/Notas";
    let notes_id = mac.add_folder(notes, "Ym9va21hcmstbm90YXM=").await?;
    mac.request("search", "files_search", json!({"query":"plan"}))
        .await?;
    mac.resolve(&[(&projects, PROJECTS), (&notes_id, notes)], Value::Null)
        .await?;
    let search = mac.helper_command("assistant_files_search")?;
    mac.command(json!({"command":"assistant_folder_remove","id":projects}))
        .await?;
    mac.answer(
        &search,
        json!({"results":[
            {"path":"/Users/ejemplo/Proyectos/plan.md","kind":"file","size_bytes":10},
            {"path":"/Users/ejemplo/Notas/plan.md","kind":"file","size_bytes":12}],
            "visited":4,"files_read":0,"bytes_read":0,"partial_files":0}),
    )
    .await?;
    let reply = mac.reply("search")?;
    let paths: Vec<&str> = reply["result"]["results"]
        .as_array()
        .context("no results")?
        .iter()
        .filter_map(|hit| hit["path"].as_str())
        .collect();
    assert_eq!(paths, ["/Users/ejemplo/Notas/plan.md"], "{reply}");
    Ok(())
}

/// The late report of a removed folder does not bring it back, and a folder
/// the second step could not open says so in Ajustes.
#[tokio::test]
async fn what_ajustes_knows_follows_the_folders_kept_and_the_second_step() -> Result<()> {
    let (mut mac, id) = mac_with_projects().await?;
    let notes = "/Users/ejemplo/Notas";
    let notes_id = mac.add_folder(notes, "Ym9va21hcmstbm90YXM=").await?;
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    mac.request("step", "files_read", json!({"path":plan}))
        .await?;
    let check = mac.helper_command("assistant_folders")?;
    mac.command(json!({"command":"assistant_folder_remove","id":notes_id}))
        .await?;
    mac.answer(
        &check,
        json!({"folders":[{"state":"accessible","id":id,"path":PROJECTS},
            {"state":"accessible","id":notes_id,"path":notes}],
            "requested":found(plan,"file",10)}),
    )
    .await?;
    assert!(!mac.runtime.features.files.access.contains_key(&notes_id));
    let read = mac.helper_command("assistant_files_read")?;
    mac.refuse(
        &read,
        "assistant_folder_unavailable",
        json!({"id":id,"cause":{"kind":"access_denied","errno":1}}),
    )
    .await?;
    let reply = mac.reply("step")?;
    assert_eq!(reply["refusal"]["code"], "folder_unavailable", "{reply}");
    assert_eq!(
        mac.folders_topic()?["value"],
        json!([{"id":id,"path":PROJECTS,
            "access":{"state":"unavailable","cause":{"kind":"access_denied","errno":1}}}])
    );
    Ok(())
}

/// A helper that is gone, not yet back, is not said to have restarted.
#[tokio::test]
async fn a_helper_that_disconnects_mid_request_is_not_running_not_restarted() -> Result<()> {
    let (mut mac, _id) = mac_with_projects().await?;
    mac.request(
        "gone",
        "files_read",
        json!({"path":"/Users/ejemplo/Proyectos/plan.md"}),
    )
    .await?;
    mac.helper_command("assistant_folders")?;
    mac.runtime
        .native_event(json!({"event":"helper_disconnected"}))
        .await?;
    let reply = mac.reply("gone")?;
    assert_eq!(reply["refusal"]["code"], "helper_not_running", "{reply}");
    Ok(())
}

/// What the screen says carries the numbers of the refusal.
#[tokio::test]
async fn a_folder_not_kept_says_its_cause_with_its_numbers() -> Result<()> {
    let mut mac = mac().await?;
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    let oversized = "A".repeat(orbit_core::assistant_files::MAX_BOOKMARK_BYTES.div_ceil(3) * 4 + 8);
    mac.answer(
        &pick,
        json!({"state":"chosen","path":PROJECTS,"bookmark_base64":oversized}),
    )
    .await?;
    let notice = mac
        .runtime
        .state
        .durable
        .notice
        .clone()
        .context("no notice at all")?;
    assert_eq!(
        serde_json::to_value(&notice)?["data"],
        json!({"code":"folder_unavailable","cause":"bookmark_too_large",
            "bytes": (orbit_core::assistant_files::MAX_BOOKMARK_BYTES.div_ceil(3) * 4 + 8) / 4 * 3,
            "max_bytes": orbit_core::assistant_files::MAX_BOOKMARK_BYTES})
    );
    // The panel's own failure keeps the system's number.
    mac.command(json!({"command":"assistant_folder_add"}))
        .await?;
    let pick = mac.helper_command("assistant_folder_pick")?;
    mac.refuse(
        &pick,
        "native_operation_failed",
        json!({"domain":"NSCocoaErrorDomain","os_status":256}),
    )
    .await?;
    let notice = mac
        .runtime
        .state
        .durable
        .notice
        .clone()
        .context("no notice at all")?;
    assert_eq!(
        serde_json::to_value(&notice)?["data"],
        json!({"code":"native_operation_failed","status":256})
    );
    Ok(())
}
