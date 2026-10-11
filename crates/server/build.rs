fn main() {
    // Re-embed the frontend after `npm run build`, including newly fingerprinted files.
    println!("cargo:rerun-if-changed=../../web/dist");
}
