fn main() {
    // Ask Windows for administrator rights on start (one UAC prompt), like Revo does.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(new_manifest("AppRemover").requested_execution_level(ExecutionLevel::RequireAdministrator))
            .expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
