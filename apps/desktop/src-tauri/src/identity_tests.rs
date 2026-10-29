#[test]
fn compiled_identity_matches_the_bundle() {
    let context: tauri::Context<tauri::test::MockRuntime> = tauri::generate_context!(test = true);
    let (name, identifier) = if cfg!(feature = "pruebas") {
        ("Orbit Pruebas", "app.orbit.companion.pruebas")
    } else {
        ("Orbit", "app.orbit.companion")
    };
    assert_eq!(context.config().product_name.as_deref(), Some(name));
    assert_eq!(context.config().identifier, identifier);
    assert_eq!(context.package_info().name, name);
}
