fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    // GPUI loads icon resource 1 for its native window class.
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("Could not embed the application icon");
}
