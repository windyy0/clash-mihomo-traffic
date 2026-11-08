fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/traffic.ico");
    println!("cargo:rerun-if-changed=assets/tray-light.ico");
    println!("cargo:rerun-if-changed=assets/tray-dark.ico");
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("Windows resource compilation failed");
}
