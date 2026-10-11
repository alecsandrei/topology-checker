fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_GDAL").is_some() {
        let root =
            std::path::PathBuf::from(std::env::var_os("DEP_PROJ_ROOT").expect("bundled PROJ root"));
        let database = root.join("share/proj/proj.db");
        println!("cargo:rerun-if-changed={}", database.display());
        let output =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"))
                .join("proj.db");
        std::fs::copy(database, &output).expect("embed bundled PROJ database");
        println!(
            "cargo:rustc-env=TOPOLOGY_PROJ_DATABASE={}",
            output.display()
        );
    }
}
