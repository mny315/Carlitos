fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cc::Build::new()
            .file("vendor/sonic/sonic.c")
            .include("vendor/sonic")
            .compile("sonic");
        println!("cargo:rerun-if-changed=vendor/sonic/sonic.c");
        println!("cargo:rerun-if-changed=vendor/sonic/sonic.h");
        windows_resources();
    }
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            .with_debug_info(std::env::var_os("CARGO_FEATURE_ANDROID_UI_TESTS").is_some()),
    )
    .unwrap();
    let translations: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&std::fs::read_to_string("data/locales/en.json").unwrap()).unwrap();
    let mut code = String::from("#[macro_export]\nmacro_rules! tformat {\n");
    for (ru, en) in translations.iter().filter(|(s, _)| s.contains('{')) {
        code.push_str(&format!("({ru:?} $(, $($args:tt)*)?) => {{ if $crate::i18n::english() {{ format!({en:?} $(, $($args)*)?) }} else {{ format!({ru:?} $(, $($args)*)?) }} }};\n"));
    }
    code.push_str("}\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("translations.rs"),
        code,
    )
    .unwrap();
    println!("cargo:rerun-if-changed=data/locales/en.json");
}

fn windows_resources() {
    use std::{fs, path::PathBuf};
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let tree = resvg::usvg::Tree::from_data(
        &fs::read("data/icons/io.github.mny315.Carlitos.svg").unwrap(),
        &resvg::usvg::Options::default(),
    )
    .unwrap();
    let mut image = resvg::tiny_skia::Pixmap::new(128, 128).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut image.as_mut(),
    );
    let png = image.encode_png().unwrap();
    // The installer uses the same icon as the application.
    let mut ico = vec![0, 0, 1, 0, 1, 0, 128, 128, 0, 0, 1, 0, 32, 0];
    ico.extend((png.len() as u32).to_le_bytes());
    ico.extend(22u32.to_le_bytes());
    ico.extend(&png);
    fs::write(out.join("carlitos.ico"), ico).unwrap();
    // A .res file can be linked directly by link.exe and lld-link. Writing the
    // standard resource records also keeps Linux cross builds independent of rc.exe.
    fn resource(output: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
        output.extend((data.len() as u32).to_le_bytes());
        output.extend(32u32.to_le_bytes());
        for word in [0xffff, kind, 0xffff, id] {
            output.extend(word.to_le_bytes());
        }
        output.extend(0u32.to_le_bytes());
        output.extend(0x1030u16.to_le_bytes());
        output.extend(0u16.to_le_bytes());
        output.extend(0u32.to_le_bytes());
        output.extend(0u32.to_le_bytes());
        output.extend(data);
        while !output.len().is_multiple_of(4) {
            output.push(0);
        }
    }
    let mut res = Vec::new();
    resource(&mut res, 0, 0, &[]);
    resource(&mut res, 3, 2, &png);
    let mut group = vec![0, 0, 1, 0, 1, 0, 128, 128, 0, 0, 1, 0, 32, 0];
    group.extend((png.len() as u32).to_le_bytes());
    group.extend(2u16.to_le_bytes());
    resource(&mut res, 14, 1, &group);
    resource(
        &mut res,
        24,
        1,
        &fs::read("data/windows/app.manifest").unwrap(),
    );
    let notices = [
        "LICENSE",
        "data/licenses/SLINT-LICENSE.md",
        "data/licenses/THIRD-PARTY.md",
        "data/licenses/DEJAVU-LICENSE.txt",
        "vendor/sonic/LICENSE",
    ]
    .map(|path| {
        println!("cargo:rerun-if-changed={path}");
        format!("{path}\n\n{}\n\n", fs::read_to_string(path).unwrap())
    })
    .join("\n");
    resource(&mut res, 10, 1, notices.as_bytes());
    fs::write(out.join("licenses.txt"), notices).unwrap();
    let path = out.join("carlitos.res");
    fs::write(&path, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", path.display());
    println!("cargo:rerun-if-changed=data/windows/app.manifest");
    println!("cargo:rerun-if-changed=data/icons/io.github.mny315.Carlitos.svg");
}
