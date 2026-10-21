//! One recovery page in synthetic stores; production retains the scan to EOF.
pub fn recover_page(
    store: &mut orbit_store::Store,
    limit: usize,
) -> orbit_store::Result<orbit_store::RecoveryReport> {
    let mut scan = store.begin_recovery()?;
    store.recover_next(&mut scan, limit)
}
