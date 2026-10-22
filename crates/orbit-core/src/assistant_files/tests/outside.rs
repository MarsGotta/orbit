//! What a path outside the folders, or missing inside one, is told: never
//! whether something exists outside, whatever case the person typed.
use super::*;

/// macOS's file system ignores case: `realpath` turns `…/proyectos/plan.md`
/// into `…/Proyectos/plan.md` when the file exists, and it is admitted. When
/// it does not exist, the same path is called outside the folders, so the
/// assistant is told it asked outside instead of «it does not exist».
#[test]
fn a_missing_file_named_in_another_case_is_missing_not_outside() {
    let asked = "/Users/ejemplo/proyectos/falta.md";
    let refusal = admit(
        Asked::Read,
        asked,
        &saved(),
        &reply(RequestedPath::NotFound),
    );
    assert_eq!(
        refusal,
        Err(Refusal::FileUnavailable {
            path: asked.into(),
            cause: FileCause::NotFound,
        })
    );
}

/// A path outside every folder answers the same whether it exists or not:
/// with `/./` in it, an existing file's refusal names where it leads.
#[test]
fn an_outside_path_that_exists_is_refused_like_one_that_does_not() {
    let existing = admit(
        Asked::Read,
        "/Users/ejemplo/Privado/./claves.txt",
        &saved(),
        &reply(found(SECRETS, EntryKind::File, 12)),
    );
    let Err(Refusal::OutsideFolders { resolved, .. }) = existing else {
        panic!("not refused as outside: {existing:?}");
    };
    assert_eq!(
        resolved, None,
        "the refusal says the file exists, and its name"
    );
}
