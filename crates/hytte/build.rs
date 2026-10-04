fn main() {
    use embed_manifest::{
        embed_manifest, manifest::DpiAwareness, manifest::MaxVersionTested, new_manifest,
    };
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let _ = embed_manifest(
            new_manifest("Hytte")
                .dpi_awareness(DpiAwareness::PerMonitorV2)
                .max_version_tested(MaxVersionTested::Windows11Version22H2),
        );
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
}
