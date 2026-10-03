fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    // GPUI loads icon resource 1 for its native window class.
    let root = std::env::var("CARGO_MANIFEST_DIR").expect("Missing crate directory");
    let icon = format!("{}/assets/app.ico", root.replace('\\', "/"));
    // llvm-rc runs from the resource's directory; use an absolute resource path
    // so native RC and the Linux cross compiler resolve the same icon.
    embed_resource::compile("assets/app.rc", [format!("APP_ICON=\"{icon}\"")])
        .manifest_required()
        .expect("Could not embed the application icon");
}
