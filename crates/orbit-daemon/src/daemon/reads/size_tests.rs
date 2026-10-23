use super::*;

#[test]
fn accumulated_page_size_matches_serialized_json_at_the_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    let cursor = Some(Cursor::StartAndId {
        start_ms: 1_800_000_000_000,
        id: "55555555-5555-4555-8555-000000000003".into(),
    });
    let answer = |items: Vec<serde_json::Value>, next: Option<Cursor>| Knowledge::Present {
        value: Page {
            items,
            next,
            total: 3,
            revision: 7,
            store_version: None,
        },
        as_of_ms: Some(1_800_000_000_000),
    };
    let ceiling = ReadRequest::AgendaRange {
        start_ms: 1_800_000_000_000,
        end_ms: 1_802_678_400_000,
        cursor: None,
    }
    .id()
    .max_bytes();
    let mut items = vec![
        serde_json::json!("alpha"),
        serde_json::json!("beta"),
        serde_json::json!(""),
    ];
    let small = serde_json::to_vec(&answer(items.clone(), cursor.clone()))?.len();
    let fill = ceiling
        .checked_sub(small)
        .ok_or("fixture already exceeds ceiling")?;
    items[2] = serde_json::json!("x".repeat(fill));
    let actual = serde_json::to_vec(&answer(items.clone(), cursor.clone()))?.len();
    assert_eq!(actual, ceiling);

    let base_bytes = serde_json::to_vec(&answer(Vec::new(), None))?.len();
    let next_bytes = serde_json::to_vec(&cursor)?.len();
    let mut accepted_item_bytes = 0;
    let mut estimated = base_bytes;
    for (index, item) in items.iter().enumerate() {
        estimated = candidate_page_bytes(
            base_bytes,
            accepted_item_bytes,
            index,
            serde_json::to_vec(item)?.len(),
            next_bytes,
        );
        accepted_item_bytes = estimated - base_bytes - (next_bytes - 4);
    }
    assert_eq!(estimated, actual);
    Ok(())
}
