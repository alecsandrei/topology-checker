//! Print the plugin manifest: `cargo run -p topology-checker-rustgis --example manifest > crates/rustgis-plugin/plugin.json`.

fn main() {
    println!("{}", topology_checker_rustgis::manifest_json());
}
