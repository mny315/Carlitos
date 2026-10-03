pub fn configure() -> anyhow::Result<()> {
    // Windows' default OpenGL driver and fresh Wine installations may not expose
    // a usable GL configuration. Keep startup independent of graphics drivers.
    let backend = std::env::var("SLINT_BACKEND")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "winit-software".into());
    slint::BackendSelector::new()
        .backend_name(backend)
        .select()?;

    // Wine can report an empty system font collection. Append a bundled fallback
    // so normal Windows fonts stay preferred and Cyrillic still works without them.
    use slint::fontique_011::{fontique, shared_collection};
    let mut collection = shared_collection();
    let fonts = collection.register_fonts(
        fontique::Blob::new(std::sync::Arc::new(
            include_bytes!("../../data/fonts/DejaVuSans.ttf").as_slice(),
        )),
        None,
    );
    anyhow::ensure!(
        !fonts.is_empty(),
        "Unable to load the bundled interface font"
    );
    for generic in [
        fontique::GenericFamily::SansSerif,
        fontique::GenericFamily::SystemUi,
        fontique::GenericFamily::UiSansSerif,
        fontique::GenericFamily::Serif,
        fontique::GenericFamily::Monospace,
    ] {
        collection.append_generic_families(generic, fonts.iter().map(|(id, _)| *id));
    }
    Ok(())
}
