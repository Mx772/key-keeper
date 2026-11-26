//! Embeds the Windows icon resource (`keykeeper.rc`) into `keykeeper.exe`. Elsewhere this is a
//! no-op: the macOS bundle gets its icon from cargo-packager.

fn main() {
    println!("cargo:rerun-if-changed=keykeeper.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if let Err(e) =
        embed_resource::compile("keykeeper.rc", embed_resource::NONE).manifest_optional()
    {
        // An icon is cosmetic: warn rather than fail builds without a resource compiler.
        println!("cargo:warning=keykeeper.rc not embedded: {e}");
    }
}
