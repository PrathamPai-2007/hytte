fn main() {
    // Icons, manifest and version info (hytte.rc). A failure here must fail the build: without
    // the manifest the pill is not DPI-aware and the CLI opens a console window on double-click.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let v = std::env::var("CARGO_PKG_VERSION").unwrap();
        let comma = format!("{},0", v.split('-').next().unwrap().replace('.', ","));
        embed_resource::compile(
            "hytte.rc",
            [
                format!("HYTTE_VER_COMMA={comma}"),
                format!("HYTTE_VER_STR=\"{v}\""),
            ],
        )
        .manifest_required()
        .unwrap();
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=hytte.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets");
}
