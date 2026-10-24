//! The folders decision with the facts a helper reports, no disk.
use super::page_fit::PAGE_ENVELOPE_BYTES;
use super::*;
use crate::refusal::Refusal;

const PROJECTS: &str = "/Users/ejemplo/Proyectos";
const NOTES: &str = "/Users/ejemplo/Notas";
const SECRETS: &str = "/Users/ejemplo/Privado/claves.txt";

fn folder(id: &str, path: &str) -> Folder {
    Folder {
        id: id.into(),
        path: path.into(),
        bookmark_base64: "Ym9va21hcms=".into(),
    }
}

fn saved() -> Vec<Folder> {
    vec![folder("projects", PROJECTS), folder("notes", NOTES)]
}

fn open(id: &str, path: &str) -> FolderReport {
    FolderReport::Accessible {
        id: id.into(),
        path: path.into(),
        renewed_bookmark_base64: None,
    }
}

fn found(path: &str, kind: EntryKind, size_bytes: u64) -> RequestedPath {
    RequestedPath::Found {
        path: path.into(),
        kind,
        size_bytes,
        modified_ms: Some(1_790_200_800_000),
    }
}

fn reply(requested: RequestedPath) -> FoldersReply {
    FoldersReply {
        folders: vec![open("projects", PROJECTS), open("notes", NOTES)],
        requested: Some(requested),
    }
}

fn both_folders() -> Vec<String> {
    vec![PROJECTS.to_owned(), NOTES.to_owned()]
}

#[test]
fn a_path_outside_every_folder_is_refused_with_it_and_the_folders() {
    let refused = admit(
        Asked::Read,
        SECRETS,
        &saved(),
        &reply(found(SECRETS, EntryKind::File, 12)),
    );
    assert_eq!(
        refused,
        Err(Refusal::OutsideFolders {
            path: SECRETS.into(),
            resolved: None,
            folders: both_folders(),
        })
    );
}

#[test]
fn a_link_inside_a_folder_that_leads_outside_is_refused_with_where_it_leads() {
    let link = "/Users/ejemplo/Proyectos/atajo.txt";
    let refused = admit(
        Asked::Read,
        link,
        &saved(),
        &reply(found(SECRETS, EntryKind::File, 12)),
    );
    assert_eq!(
        refused,
        Err(Refusal::OutsideFolders {
            path: link.into(),
            resolved: Some(SECRETS.into()),
            folders: both_folders(),
        })
    );
}

#[test]
fn dot_dot_out_of_a_folder_is_refused_even_before_it_is_resolved() {
    let climbing = "/Users/ejemplo/Proyectos/../Privado/claves.txt";
    assert_eq!(lexical(climbing), SECRETS);
    assert!(!is_within(&lexical(climbing), PROJECTS));
    let refused = admit(
        Asked::Read,
        climbing,
        &saved(),
        &reply(found(SECRETS, EntryKind::File, 12)),
    );
    // Written outside a folder: «outside», never where it leads, which
    // would say that it exists and its true name.
    assert_eq!(
        refused,
        Err(Refusal::OutsideFolders {
            path: climbing.into(),
            resolved: None,
            folders: both_folders(),
        })
    );
    // Missing and outside: the same refusal, never whether it exists.
    let missing = admit(
        Asked::Read,
        climbing,
        &saved(),
        &reply(RequestedPath::NotFound),
    );
    assert_eq!(
        missing,
        Err(Refusal::OutsideFolders {
            path: climbing.into(),
            resolved: None,
            folders: both_folders(),
        })
    );
}

#[test]
fn a_sibling_whose_name_starts_like_a_folder_is_not_inside_it() {
    let sibling = "/Users/ejemplo/Proyectos-viejos/plan.md";
    assert!(!is_within(sibling, PROJECTS));
    assert!(is_within(PROJECTS, PROJECTS));
    assert!(is_within(
        "/Users/ejemplo/Proyectos/plan.md",
        "/Users/ejemplo/Proyectos/"
    ));
    assert!(is_within("/Volumes/Disco/a", "/"));
    let refused = admit(
        Asked::Read,
        sibling,
        &saved(),
        &reply(found(sibling, EntryKind::File, 10)),
    );
    assert!(matches!(refused, Err(Refusal::OutsideFolders { .. })));
}

#[test]
fn inside_a_folder_the_canonical_path_and_its_folder_are_admitted() {
    let plan = "/Users/ejemplo/Proyectos/./informes//plan.md";
    let canonical = "/Users/ejemplo/Proyectos/informes/plan.md";
    let admitted = admit(
        Asked::Read,
        plan,
        &saved(),
        &reply(found(canonical, EntryKind::File, 2_048)),
    );
    assert_eq!(
        admitted,
        Ok(Admitted {
            folder: "projects".into(),
            path: canonical.into(),
            size_bytes: 2_048,
            modified_ms: Some(1_790_200_800_000),
            content: Some(ContentKind::Text),
        })
    );
    // A link inside that stays inside is followed.
    let link = admit(
        Asked::Read,
        "/Users/ejemplo/Notas/enlace.md",
        &saved(),
        &reply(found(canonical, EntryKind::File, 2_048)),
    );
    assert_eq!(link.map(|admitted| admitted.folder), Ok("projects".into()));
}

#[test]
fn nested_folders_belong_to_the_innermost() {
    let saved = vec![folder("outer", "/Users/ejemplo"), folder("inner", PROJECTS)];
    let reply = FoldersReply {
        folders: vec![open("outer", "/Users/ejemplo"), open("inner", PROJECTS)],
        requested: Some(found("/Users/ejemplo/Proyectos/a.txt", EntryKind::File, 1)),
    };
    let admitted = admit(
        Asked::Read,
        "/Users/ejemplo/Proyectos/a.txt",
        &saved,
        &reply,
    );
    assert_eq!(admitted.map(|admitted| admitted.folder), Ok("inner".into()));
}

#[test]
fn a_missing_path_inside_is_not_found_and_says_its_path() {
    let missing = "/Users/ejemplo/Proyectos/no-existe.md";
    assert_eq!(
        admit(
            Asked::Read,
            missing,
            &saved(),
            &reply(RequestedPath::NotFound)
        ),
        Err(Refusal::FileUnavailable {
            path: missing.into(),
            cause: FileCause::NotFound,
        })
    );
}

#[test]
fn a_file_too_large_for_its_kind_is_refused_with_both_sizes() {
    let pdf = "/Users/ejemplo/Proyectos/manual.PDF";
    assert_eq!(content_kind(pdf), ContentKind::Pdf);
    assert_eq!(
        admit(
            Asked::Read,
            pdf,
            &saved(),
            &reply(found(pdf, EntryKind::File, MAX_PDF_BYTES + 1)),
        ),
        Err(Refusal::FileTooLarge {
            path: pdf.into(),
            content: ContentKind::Pdf,
            bytes: MAX_PDF_BYTES + 1,
            max_bytes: MAX_PDF_BYTES,
        })
    );
    let photo = "/Users/ejemplo/Notas/foto.heic";
    assert_eq!(
        admit(
            Asked::Read,
            photo,
            &saved(),
            &reply(found(photo, EntryKind::File, MAX_IMAGE_BYTES + 1)),
        ),
        Err(Refusal::FileTooLarge {
            path: photo.into(),
            content: ContentKind::Image,
            bytes: MAX_IMAGE_BYTES + 1,
            max_bytes: MAX_IMAGE_BYTES,
        })
    );
    // At the ceiling, it is read.
    assert!(
        admit(
            Asked::Read,
            pdf,
            &saved(),
            &reply(found(pdf, EntryKind::File, MAX_PDF_BYTES)),
        )
        .is_ok()
    );
    // Text has no size ceiling: it is read one page at a time.
    let log = "/Users/ejemplo/Proyectos/registro.log";
    let admitted = admit(
        Asked::Read,
        log,
        &saved(),
        &reply(found(log, EntryKind::File, 10 * MAX_PDF_BYTES)),
    );
    assert_eq!(
        admitted.map(|admitted| admitted.content),
        Ok(Some(ContentKind::Text))
    );
}

#[test]
fn a_folder_whose_bookmark_no_longer_opens_says_so_for_paths_inside_it() {
    let cause = FolderCause::BookmarkUnresolvable {
        domain: "NSCocoaErrorDomain".into(),
        code: 4,
    };
    let reply = FoldersReply {
        folders: vec![
            open("projects", PROJECTS),
            FolderReport::Unavailable {
                id: "notes".into(),
                cause: cause.clone(),
            },
        ],
        requested: Some(RequestedPath::NotFound),
    };
    assert_eq!(
        admit(
            Asked::Read,
            "/Users/ejemplo/Notas/lista.md",
            &saved(),
            &reply
        ),
        Err(Refusal::FolderUnavailable {
            path: NOTES.into(),
            cause,
        })
    );
    // The refusal for a path outside still names the broken folder, at its
    // last known path.
    assert_eq!(
        folder_paths(&saved(), &reply.folders),
        vec![PROJECTS.to_owned(), NOTES.to_owned()]
    );
}

#[test]
fn reading_a_folder_or_listing_a_file_names_what_it_is() {
    assert_eq!(
        admit(
            Asked::Read,
            PROJECTS,
            &saved(),
            &reply(found(PROJECTS, EntryKind::Folder, 0)),
        ),
        Err(Refusal::FileUnavailable {
            path: PROJECTS.into(),
            cause: FileCause::NotAFile,
        })
    );
    let plan = "/Users/ejemplo/Proyectos/plan.md";
    assert_eq!(
        admit(
            Asked::List,
            plan,
            &saved(),
            &reply(found(plan, EntryKind::File, 1)),
        ),
        Err(Refusal::FileUnavailable {
            path: plan.into(),
            cause: FileCause::NotAFolder,
        })
    );
    assert_eq!(
        admit(
            Asked::Search,
            PROJECTS,
            &saved(),
            &reply(found(PROJECTS, EntryKind::Folder, 0)),
        )
        .map(|admitted| admitted.content),
        Ok(None)
    );
}

#[test]
fn a_requested_path_must_be_absolute_and_bounded() {
    assert!(check_requested_path(PROJECTS).is_ok());
    for path in ["Proyectos/plan.md", "", "/a\0b"] {
        assert!(
            matches!(
                check_requested_path(path),
                Err(Refusal::InvalidParameter {
                    expected: crate::refusal::Expected::AbsolutePath { .. },
                    ..
                })
            ),
            "{path:?}"
        );
    }
    let long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
    assert_eq!(
        check_requested_path(&long),
        Err(Refusal::InvalidParameter {
            parameter: crate::refusal::Parameter::Request,
            expected: crate::refusal::Expected::AbsolutePath {
                bytes: (MAX_PATH_BYTES + 1) as u64,
                max_bytes: MAX_PATH_BYTES as u64,
            },
        })
    );
}

#[test]
fn choosing_a_folder_twice_changes_nothing_and_a_ninth_is_refused_with_counts() {
    let folders = saved();
    assert_eq!(with_folder(&folders, folder("again", PROJECTS)), Ok(None));
    let Ok(Some(three)) = with_folder(&folders, folder("third", "/Users/ejemplo/Fotos")) else {
        panic!("a third folder was not added");
    };
    assert_eq!(three.len(), 3);
    let full: Vec<Folder> = (0..MAX_FOLDERS)
        .map(|index| folder(&format!("f{index}"), &format!("/Users/ejemplo/C{index}")))
        .collect();
    let refusal = Refusal::TooMany {
        resource: crate::refusal::Resource::Folder,
        count: MAX_FOLDERS as u64 + 1,
        max: MAX_FOLDERS as u64,
    };
    assert_eq!(room_for_another(&full), Err(refusal.clone()));
    assert_eq!(
        with_folder(&full, folder("ninth", "/Users/ejemplo/Otra")),
        Err(refusal)
    );
    assert_eq!(without_folder(&three, "third"), Some(folders.clone()));
    assert_eq!(without_folder(&folders, "unknown"), None);
}

#[test]
fn a_bookmark_over_the_ceiling_is_refused_with_its_size() {
    let mut large = folder("large", PROJECTS);
    large.bookmark_base64 = "A".repeat((MAX_BOOKMARK_BYTES + 3) / 3 * 4);
    let Err(Refusal::FolderUnavailable {
        cause: FolderCause::BookmarkTooLarge { bytes, max_bytes },
        ..
    }) = with_folder(&[], large)
    else {
        panic!("an oversized bookmark was kept");
    };
    assert!(bytes > MAX_BOOKMARK_BYTES as u64);
    assert_eq!(max_bytes, MAX_BOOKMARK_BYTES as u64);
}

#[test]
fn a_renewed_bookmark_and_a_moved_folder_are_kept() {
    let reports = vec![
        FolderReport::Accessible {
            id: "projects".into(),
            path: "/Users/ejemplo/Trabajo/Proyectos".into(),
            renewed_bookmark_base64: Some("bnVldm8=".into()),
        },
        open("notes", NOTES),
    ];
    let Some(folders) = refreshed(&saved(), &reports) else {
        panic!("nothing was refreshed");
    };
    assert_eq!(folders[0].path, "/Users/ejemplo/Trabajo/Proyectos");
    assert_eq!(folders[0].bookmark_base64, "bnVldm8=");
    assert_eq!(folders[1], saved()[1]);
    assert_eq!(
        refreshed(
            &saved(),
            &[open("projects", PROJECTS), open("notes", NOTES)]
        ),
        None
    );
}

#[test]
fn a_text_prefix_fits_its_encoded_budget_at_a_character_boundary() {
    let text = "línea \"uno\"\nlínea dos";
    assert_eq!(fitting_prefix(text, encoded_len(text)), text.len());
    let cut = fitting_prefix(text, 10);
    assert!(text.is_char_boundary(cut));
    assert!(encoded_len(&text[..cut]) <= 10);
    // Control characters cost six bytes once escaped.
    assert_eq!(encoded_len("\u{01}"), 8);
    assert_eq!(
        serde_json::to_string("\u{01}a\"\\\n")
            .map(|encoded| encoded.len())
            .ok(),
        Some(encoded_len("\u{01}a\"\\\n"))
    );
    assert_eq!(fitting_prefix("abc", 2), 0);
}

#[test]
fn every_bound_says_what_it_protects_in_its_numbers() {
    // The text page fits the answer budget of the translator, which is the
    // shared constant; the page bytes clear the hub frame with room.
    assert!(MAX_TEXT_PAGE_BYTES as usize <= MAX_PAGE_BYTES);
    const { assert!(MAX_PAGE_BYTES + 32 * 1024 <= crate::limits::MAX_HUB_OUTGOING_FRAME_BYTES) };
    const { assert!(SEARCH_DEADLINE_MS * 2 <= HELPER_REQUEST_TIMEOUT_MS) };
    const { assert!(HELPER_REQUEST_TIMEOUT_MS < 30_000) };
    const { assert!(MAX_QUERY_BYTES > 255) };
}

fn entry(index: usize) -> Entry {
    Entry {
        name: format!("documento-{index:04}.md"),
        kind: EntryKind::File,
        size_bytes: Some(1_024),
        modified_ms: Some(1_790_200_800_000),
        unavailable: None,
    }
}

#[test]
fn a_listing_page_fits_its_budget_and_says_where_it_goes_on() {
    let entries: Vec<Entry> = (0..200).map(entry).collect();
    let one = serde_json::to_vec(&entries[0]).map_or(0, |bytes| bytes.len()) + 1;
    let (kept, next) = fit_entries(entries.clone(), 400, Some(600), one * 10);
    assert_eq!(kept.len(), 10);
    assert_eq!(next, Some(410));
    // Everything fits: the helper's own continuation stands.
    let (kept, next) = fit_entries(entries, 400, Some(600), MAX_PAGE_BYTES);
    assert_eq!(kept.len(), 200);
    assert_eq!(next, Some(600));
    // One entry larger than the page still travels, so the walk moves.
    let (kept, next) = fit_entries(vec![entry(1), entry(2)], 0, None, 1);
    assert_eq!(kept.len(), 1);
    assert_eq!(next, Some(1));
}

#[test]
fn a_search_cut_by_its_page_says_so() {
    let hit = |index: usize| SearchHit {
        path: format!("/Users/ejemplo/Proyectos/nota-{index}.md"),
        kind: EntryKind::File,
        size_bytes: Some(10),
        line: Some(3),
        snippet: Some("presupuesto del trimestre".into()),
    };
    let hits: Vec<SearchHit> = (0..50).map(hit).collect();
    let (kept, stopped) = fit_hits(hits.clone(), None, MAX_PAGE_BYTES);
    assert_eq!((kept.len(), stopped), (50, None));
    let (kept, stopped) = fit_hits(hits, None, 500);
    assert!(kept.len() < 50);
    assert_eq!(
        stopped,
        Some(SearchStop {
            cause: SearchLimit::PageBytes,
            limit: 500,
        })
    );
}

#[test]
fn a_text_page_cut_by_its_budget_continues_after_what_it_carried() {
    let text = "ñ".repeat(1_000);
    let (kept, next) = fit_text(text.clone(), 96_000, None, 502);
    assert_eq!(kept.chars().count(), 250);
    assert_eq!(next, Some(96_000 + 500));
    let (kept, next) = fit_text(text.clone(), 0, Some(2_000), MAX_PAGE_BYTES);
    assert_eq!((kept, next), (text, Some(2_000)));
}

#[test]
fn pdf_pages_fit_whole_and_the_last_is_cut_inside_its_text() {
    let page = |page: u32, length: usize| PdfPage {
        page,
        text: "a".repeat(length),
    };
    // Page 3 (index 2) starts 100 bytes into its text.
    let pages = vec![page(3, 400), page(4, 400), page(5, 400)];
    let budget = 2 * (402 + PAGE_ENVELOPE_BYTES) + 200;
    let (kept, next) = fit_pages(pages.clone(), 2, 100, None, budget);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[2].text.len(), 200 - 2 - PAGE_ENVELOPE_BYTES);
    assert_eq!(next, Some((4, (200 - 2 - PAGE_ENVELOPE_BYTES) as u64)));
    // Cut in the first page: the offset adds to where it started.
    let (kept, next) = fit_pages(pages.clone(), 2, 100, None, 100);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        next,
        Some((2, 100 + (100 - 2 - PAGE_ENVELOPE_BYTES) as u64))
    );
    // All fit: the helper's own continuation stands.
    let (kept, next) = fit_pages(pages, 2, 100, Some((5, 0)), MAX_PAGE_BYTES);
    assert_eq!((kept.len(), next), (3, Some((5, 0))));
}

#[test]
fn the_extensions_decide_how_a_file_is_read() {
    assert_eq!(content_kind("/a/b/Informe.pdf"), ContentKind::Pdf);
    for image in ["/a/foto.JPG", "/a/b.png", "/a/c.heic", "/a/d.tiff"] {
        assert_eq!(content_kind(image), ContentKind::Image, "{image}");
    }
    for text in ["/a/notas.md", "/a/datos.csv", "/a/sin-extension", "/a.b/c"] {
        assert_eq!(content_kind(text), ContentKind::Text, "{text}");
    }
    assert_eq!(max_bytes(ContentKind::Text), None);
}

/// The helper's replies, as `contracts/fixtures/native/results/` pins them,
/// decode into the shapes the daemon reads; Swift encodes the same shapes.
#[test]
fn the_native_result_fixtures_decode_into_the_replies() -> Result<(), serde_json::Error> {
    let pick: PickReply = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/assistant_folder_pick.json"
    ))?;
    assert!(matches!(pick, PickReply::Chosen { .. }));
    let folders: FoldersReply = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/assistant_folders.json"
    ))?;
    assert_eq!(folders.folders.len(), 2);
    assert!(matches!(
        folders.requested,
        Some(RequestedPath::Found { .. })
    ));
    let list: ListReply = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/assistant_files_list.json"
    ))?;
    assert_eq!(list.entries[2].kind, EntryKind::Link);
    let search: SearchReply = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/assistant_files_search.json"
    ))?;
    assert_eq!(search.results[0].line, Some(3));
    let read: ReadReply = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/assistant_files_read.json"
    ))?;
    let ReadReply::Text {
        text, size_bytes, ..
    } = read
    else {
        panic!("the read fixture is not text");
    };
    assert_eq!(text.len() as u64, size_bytes);
    Ok(())
}

#[test]
fn a_folder_chosen_through_another_path_to_the_same_place_is_kept_once() {
    let through_link = folder("again", "/tmp/Proyectos");
    let saved = vec![folder("projects", "/private/tmp/Proyectos"), through_link];
    let Some(folders) = refreshed(
        &saved,
        &[
            open("projects", "/private/tmp/Proyectos"),
            open("again", "/private/tmp/Proyectos"),
        ],
    ) else {
        panic!("the second path to the same folder was kept");
    };
    assert_eq!(folders, vec![folder("projects", "/private/tmp/Proyectos")]);
}

#[test]
fn a_relative_path_from_the_panel_is_an_invalid_reply_not_a_long_one() {
    assert_eq!(
        check_folder(&folder("relative", "Proyectos")),
        Err(Refusal::HelperReplyInvalid {
            field: "assistant_folder_pick.path".into(),
            expected: "absolute_path".into(),
            received_type: "relative_path".into(),
        })
    );
}

mod outside;
