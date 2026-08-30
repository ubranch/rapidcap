fn main() {
    // The icon and manifest are a Windows PE resource; every other platform
    // carries its icon in the bundle instead, so there is nothing to embed.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=app.rc");
        println!("cargo:rerun-if-changed=assets/rapidcap.ico");
        // Not `unwrap_or` anything: an exe that silently ships without its icon
        // looks like a broken build, and the failure is a missing resource
        // compiler, which no fallback can paper over.
        embed_resource::compile("app.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed app.rc");
    }
}
